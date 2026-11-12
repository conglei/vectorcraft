//! A picture of a stitch plan: realistic thread or a density map, with optional overlays for
//! jumps, needle points and machine functions. Pure software rendering into an RGBA buffer, so it
//! runs the same in the app, the CLI and the browser.
//!
//! Each stitch is a capsule (a line with round ends) of thread width from needle point to needle
//! point. It is shaded as a lit cylinder: the cross-section's normal comes from the offset of the
//! pixel to the stitch's centre line, lit from the top left with a diffuse and a specular term, and
//! darkened towards both needle points where the thread goes into the fabric. A soft shadow under
//! each stitch separates it from the stitches below. Later stitches draw over earlier ones.
//!
//! Only consecutive stitches are sewn thread: a jump, trim, stop or colour change breaks the run.
//! Jumps show as dashed lines when asked for.

use crate::{Pt, Step, StitchPlan, UNITS_PER_MM};

/// The default pixels per millimetre.
pub const DEFAULT_SCALE: f64 = 10.0;
/// The smallest and largest scale, in pixels per millimetre.
pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 50.0;
/// The default thread width in mm (a 40 weight thread).
pub const DEFAULT_THREAD_MM: f64 = 0.4;
/// The thinnest and thickest thread width, in mm.
pub const MIN_THREAD_MM: f64 = 0.1;
pub const MAX_THREAD_MM: f64 = 2.0;
/// The widest or tallest picture, in pixels.
pub const MAX_SIDE: u32 = 8192;
/// The fabric around the stitches, in mm.
pub const MARGIN_MM: f64 = 2.0;
/// The default fabric: a light, warm neutral.
pub const DEFAULT_FABRIC: [u8; 3] = [236, 232, 222];
/// The density map's cell side in mm, unless the design is so large that more than
/// [`MAX_DENSITY_CELLS`] cells would be needed along a side; then cells grow to fit.
pub const DENSITY_CELL_MM: f64 = 1.0;
/// The most density cells along either side.
pub const MAX_DENSITY_CELLS: usize = 2048;
/// The coverage from which a cell counts as too dense: thread laid about twice over the cell.
/// Coverage is stitched length × thread width / cell area, so a fill with rows one thread width
/// apart has coverage 1.
pub const TOO_DENSE: f64 = 2.0;

/// What the picture shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewMode {
    /// Lit thread on fabric.
    #[default]
    Realistic,
    /// Cells coloured by thread coverage: pale green (little), green (one layer), yellow, red
    /// ([`TOO_DENSE`] and above).
    Density,
}

/// How to draw the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewOptions {
    /// Pixels per millimetre, [`MIN_SCALE`]–[`MAX_SCALE`].
    pub scale: f64,
    /// Thread width in mm, [`MIN_THREAD_MM`]–[`MAX_THREAD_MM`].
    pub thread_mm: f64,
    /// The fabric colour.
    pub fabric: [u8; 3],
    /// Jumps as thin dashed lines.
    pub show_jumps: bool,
    /// A small dot at each needle point.
    pub show_needles: bool,
    /// Rings at trims (red), colour changes (in the new thread's colour) and stops (orange).
    pub show_functions: bool,
    pub mode: PreviewMode,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        PreviewOptions {
            scale: DEFAULT_SCALE,
            thread_mm: DEFAULT_THREAD_MM,
            fabric: DEFAULT_FABRIC,
            show_jumps: false,
            show_needles: false,
            show_functions: false,
            mode: PreviewMode::Realistic,
        }
    }
}

/// A rendered preview: opaque RGBA rows, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The highest cell coverage (density mode; 0 otherwise).
    pub max_coverage: f64,
    /// The cells at or above [`TOO_DENSE`] (density mode; 0 otherwise).
    pub dense_cells: usize,
}

/// Thread coverage per cell of a grid over the design.
#[derive(Clone, Debug, PartialEq)]
pub struct DensityGrid {
    /// The cell side in mm.
    pub cell_mm: f64,
    /// The grid's top-left corner in mm.
    pub origin_mm: (f64, f64),
    pub cols: usize,
    pub rows: usize,
    /// Row by row: stitched length × thread width / cell area.
    pub coverage: Vec<f64>,
}

impl DensityGrid {
    /// The coverage of the cell at (`col`, `row`), if there is one.
    pub fn at(&self, col: usize, row: usize) -> Option<f64> {
        if col >= self.cols {
            return None;
        }
        self.coverage.get(row.checked_mul(self.cols)?.checked_add(col)?).copied()
    }

    /// The coverage of the cell that holds (`x`, `y`) mm.
    pub fn at_mm(&self, x: f64, y: f64) -> Option<f64> {
        let (c, r) = (((x - self.origin_mm.0) / self.cell_mm).floor(), ((y - self.origin_mm.1) / self.cell_mm).floor());
        if !(c.is_finite() && r.is_finite() && c >= 0.0 && r >= 0.0) || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        // Non-negative and below the grid's size, so the casts are exact.
        self.at(c as usize, r as usize)
    }
}

/// The plan's extents in mm: (left, top, right, bottom) of every stitch and jump.
fn extents(plan: &StitchPlan) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(i32, i32, i32, i32)> = None;
    for p in plan.points() {
        b = Some(match b {
            None => (p.x, p.y, p.x, p.y),
            Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        });
    }
    let (x0, y0, x1, y1) = b?;
    let mm = |v: i32| f64::from(v) / UNITS_PER_MM;
    Some((mm(x0), mm(y0), mm(x1), mm(y1)))
}

fn check(o: &PreviewOptions) -> Result<(), String> {
    if !(o.scale.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(&o.scale)) {
        return Err(format!("scale must be {MIN_SCALE}–{MAX_SCALE} pixels per mm, not {}", o.scale));
    }
    if !(o.thread_mm.is_finite() && (MIN_THREAD_MM..=MAX_THREAD_MM).contains(&o.thread_mm)) {
        return Err(format!("threadWidth must be {MIN_THREAD_MM}–{MAX_THREAD_MM} mm, not {}", o.thread_mm));
    }
    Ok(())
}

/// Where the picture sits over the plan.
#[derive(Clone, Copy, Debug)]
struct Layout {
    /// The picture's top-left corner in mm.
    left: f64,
    top: f64,
    scale: f64,
    width: u32,
    height: u32,
}

impl Layout {
    fn new(plan: &StitchPlan, o: &PreviewOptions) -> Result<Layout, String> {
        check(o)?;
        let (x0, y0, x1, y1) = extents(plan).ok_or("the design has no stitches to preview")?;
        let side = |lo: f64, hi: f64| -> Result<u32, String> {
            let px = ((hi - lo + 2.0 * MARGIN_MM) * o.scale).ceil().max(1.0);
            if !px.is_finite() || px > f64::from(MAX_SIDE) {
                return Err(format!("the preview would be {px} pixels across, more than {MAX_SIDE}: lower scale (the design is {:.1} mm)", hi - lo));
            }
            // Finite and within 1..=MAX_SIDE, so the cast is exact.
            Ok(px as u32)
        };
        Ok(Layout { left: x0 - MARGIN_MM, top: y0 - MARGIN_MM, scale: o.scale, width: side(x0, x1)?, height: side(y0, y1)? })
    }

    /// The point in pixels.
    fn px(&self, p: Pt) -> (f64, f64) {
        let (x, y) = p.to_mm();
        ((x - self.left) * self.scale, (y - self.top) * self.scale)
    }
}

/// The size `preview` would make, without drawing.
pub fn preview_size(plan: &StitchPlan, o: &PreviewOptions) -> Result<(u32, u32), String> {
    let l = Layout::new(plan, o)?;
    Ok((l.width, l.height))
}

/// An RGBA picture, opaque.
struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32, fill: [u8; 3]) -> Result<Canvas, String> {
        let (w, h) = (width as usize, height as usize);
        let n = w.checked_mul(h).and_then(|n| n.checked_mul(4)).ok_or("the preview is too large")?;
        let mut px = Vec::new();
        px.try_reserve_exact(n).map_err(|_| format!("not enough memory for a {width} × {height} preview"))?;
        let [r, g, b] = fill;
        for _ in 0..w * h {
            px.extend_from_slice(&[r, g, b, 255]);
        }
        Ok(Canvas { w, h, px })
    }

    /// The pixel indices whose centres may lie within `lo..hi`, clipped to `n`.
    fn span(lo: f64, hi: f64, n: usize) -> std::ops::Range<usize> {
        let clip = |v: f64| if v.is_finite() { v.clamp(0.0, n as f64) } else { 0.0 };
        // Clamped to 0..=n, so the casts are exact.
        (clip(lo.floor()) as usize)..(clip(hi.ceil()) as usize)
    }

    /// Blend `rgb` (0–1) over the pixel at (`x`, `y`) with alpha `a`.
    fn blend(&mut self, x: usize, y: usize, rgb: [f64; 3], a: f64) {
        if a <= 0.0 || x >= self.w {
            return;
        }
        let a = a.min(1.0);
        let Some(i) = y.checked_mul(self.w).and_then(|i| i.checked_add(x)).and_then(|i| i.checked_mul(4)) else { return };
        let Some(p) = self.px.get_mut(i..i + 3) else { return };
        for (c, s) in p.iter_mut().zip(rgb) {
            let v = f64::from(*c) * (1.0 - a) + s.clamp(0.0, 1.0) * 255.0 * a;
            // Within 0..=255 by construction.
            *c = v.round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Run `paint` over every pixel near the capsule from `a` to `b` of radius `r` (pixels). It
    /// gets the offset from the closest point of the centre line divided by `r`, the distance to
    /// the nearer end along the line in pixels, and returns a colour and an opacity, which is
    /// multiplied by the pixel's coverage.
    fn capsule(&mut self, a: (f64, f64), b: (f64, f64), r: f64, mut paint: impl FnMut((f64, f64), f64) -> ([f64; 3], f64)) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let len = len2.sqrt();
        let reach = r + 1.0;
        let xs = Canvas::span(a.0.min(b.0) - reach, a.0.max(b.0) + reach, self.w);
        let ys = Canvas::span(a.1.min(b.1) - reach, a.1.max(b.1) + reach, self.h);
        for y in ys {
            let cy = y as f64 + 0.5;
            for x in xs.clone() {
                let cx = x as f64 + 0.5;
                let t = if len2 > 0.0 { (((cx - a.0) * dx + (cy - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (ox, oy) = (cx - (a.0 + t * dx), cy - (a.1 + t * dy));
                let cover = (r + 0.5 - ox.hypot(oy)).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let (rgb, alpha) = paint((ox / r, oy / r), (t * len).min((1.0 - t) * len));
                self.blend(x, y, rgb, alpha * cover);
            }
        }
    }

    /// A ring of radius `radius` and line width `width` around `c`, or a disc when `filled`.
    fn ring(&mut self, c: (f64, f64), radius: f64, width: f64, rgb: [f64; 3], filled: bool) {
        let reach = radius + width + 1.0;
        let xs = Canvas::span(c.0 - reach, c.0 + reach, self.w);
        let ys = Canvas::span(c.1 - reach, c.1 + reach, self.h);
        for y in ys {
            for x in xs.clone() {
                let d = (x as f64 + 0.5 - c.0).hypot(y as f64 + 0.5 - c.1);
                let edge = if filled { radius + 0.5 - d } else { width / 2.0 + 0.5 - (d - radius).abs() };
                self.blend(x, y, rgb, edge.clamp(0.0, 1.0));
            }
        }
    }

    fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, rgb: [f64; 3], a: f64) {
        for y in Canvas::span(y0, y1, self.h) {
            for x in Canvas::span(x0, x1, self.w) {
                // Coverage of the pixel by the rectangle, for its fractional edges.
                let cx = ((x as f64 + 1.0).min(x1) - (x as f64).max(x0)).clamp(0.0, 1.0);
                let cy = ((y as f64 + 1.0).min(y1) - (y as f64).max(y0)).clamp(0.0, 1.0);
                self.blend(x, y, rgb, a * cx * cy);
            }
        }
    }
}

fn unit_rgb([r, g, b]: [u8; 3]) -> [f64; 3] {
    [f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0]
}

/// The light, from the top left and in front: (x right, y down, z towards the viewer), unit length.
const LIGHT: [f64; 3] = [-0.408_248, -0.408_248, 0.816_497];
/// The half vector between the light and the viewer (0, 0, 1), unit length.
const HALF: [f64; 3] = [-0.214_186, -0.214_186, 0.953_020];

/// The colour of thread `rgb` at cross-section offset `n` (offset / radius) and `end` pixels from
/// the nearer needle point, for a thread of radius `r` pixels.
fn lit(rgb: [f64; 3], n: (f64, f64), end: f64, r: f64) -> [f64; 3] {
    let flat = (n.0 * n.0 + n.1 * n.1).min(1.0);
    let nz = (1.0 - flat).sqrt();
    let diffuse = (n.0 * LIGHT[0] + n.1 * LIGHT[1] + nz * LIGHT[2]).max(0.0);
    let spec = (n.0 * HALF[0] + n.1 * HALF[1] + nz * HALF[2]).max(0.0).powi(24) * 0.35;
    // The thread dips into the fabric at each needle point.
    let sink = 0.7 + 0.3 * (end / (1.5 * r).max(0.5)).min(1.0);
    let shade = (0.3 + 0.8 * diffuse) * sink;
    rgb.map(|c| c * shade + spec * sink)
}

/// Every sewn stitch: (from, to, block index). A run breaks at anything but a stitch.
fn for_each_stitch(plan: &StitchPlan, mut f: impl FnMut(Pt, Pt, usize)) {
    for (i, b) in plan.blocks.iter().enumerate() {
        let mut prev: Option<Pt> = None;
        for s in &b.steps {
            match s {
                Step::Stitch(p) => {
                    if let Some(q) = prev {
                        f(q, *p, i);
                    }
                    prev = Some(*p);
                }
                Step::Jump(_) | Step::Trim | Step::Stop => prev = None,
            }
        }
    }
}

fn draw_realistic(c: &mut Canvas, plan: &StitchPlan, l: &Layout, o: &PreviewOptions) {
    let r = (o.thread_mm * l.scale / 2.0).max(0.25);
    let colours: Vec<[f64; 3]> = plan.blocks.iter().map(|b| unit_rgb(b.thread.rgb)).collect();
    let shadow = (0.15 * r, 0.3 * r);
    for_each_stitch(plan, |a, b, i| {
        let rgb = colours.get(i).copied().unwrap_or([0.0; 3]);
        let (pa, pb) = (l.px(a), l.px(b));
        // A soft shadow down and to the right, then the thread.
        let (sa, sb) = ((pa.0 + shadow.0, pa.1 + shadow.1), (pb.0 + shadow.0, pb.1 + shadow.1));
        c.capsule(sa, sb, r * 1.15, |n, _| ([0.0; 3], 0.22 * (1.0 - (n.0 * n.0 + n.1 * n.1).min(1.0))));
        c.capsule(pa, pb, r, |n, end| (lit(rgb, n, end, r), 1.0));
    });
}

/// Distribute the length of each stitch over the cells it crosses.
pub fn density(plan: &StitchPlan, thread_mm: f64) -> Result<DensityGrid, String> {
    let (x0, y0, x1, y1) = extents(plan).ok_or("the design has no stitches to preview")?;
    let span = (x1 - x0).max(y1 - y0);
    let cell_mm = DENSITY_CELL_MM.max(span / (MAX_DENSITY_CELLS - 1) as f64);
    let count = |lo: f64, hi: f64| -> usize {
        // At most MAX_DENSITY_CELLS by the choice of cell_mm.
        (((hi - lo) / cell_mm).floor() as usize + 1).min(MAX_DENSITY_CELLS)
    };
    let (cols, rows) = (count(x0, x1), count(y0, y1));
    let mut grid = DensityGrid { cell_mm, origin_mm: (x0, y0), cols, rows, coverage: vec![0.0; cols * rows] };
    let per_mm = thread_mm / (cell_mm * cell_mm);
    for_each_stitch(plan, |a, b, _| {
        let ((ax, ay), (bx, by)) = (a.to_mm(), b.to_mm());
        let len = (bx - ax).hypot(by - ay);
        // Pieces of at most half a cell, each counted in the cell holding its middle. A stitch is
        // at most a few cells long in a prepared plan; the cap keeps a hostile one bounded.
        let pieces = (len / (cell_mm / 2.0)).ceil().clamp(1.0, 4096.0);
        // Within 1..=4096, so the cast is exact.
        let n = pieces as usize;
        for k in 0..n {
            let t = (k as f64 + 0.5) / pieces;
            let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
            let c = ((x - x0) / cell_mm).floor().clamp(0.0, (cols - 1) as f64) as usize;
            let r = ((y - y0) / cell_mm).floor().clamp(0.0, (rows - 1) as f64) as usize;
            if let Some(v) = grid.coverage.get_mut(r * cols + c) {
                *v += len / pieces * per_mm;
            }
        }
    });
    Ok(grid)
}

/// The density ramp: pale green at none, green at one layer, yellow at 1.5, red at [`TOO_DENSE`].
fn ramp(v: f64) -> [f64; 3] {
    const STOPS: [(f64, [u8; 3]); 4] = [(0.0, [200, 232, 200]), (1.0, [60, 170, 75]), (1.5, [240, 200, 40]), (TOO_DENSE, [215, 40, 40])];
    let mut prev = STOPS[0];
    for s in STOPS {
        if v <= s.0 {
            let t = if s.0 > prev.0 { (v - prev.0) / (s.0 - prev.0) } else { 1.0 };
            let (a, b) = (unit_rgb(prev.1), unit_rgb(s.1));
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        }
        prev = s;
    }
    unit_rgb(prev.1)
}

fn draw_density(c: &mut Canvas, plan: &StitchPlan, l: &Layout, o: &PreviewOptions) -> Result<(f64, usize), String> {
    let g = density(plan, o.thread_mm)?;
    let (mut max, mut dense) = (0.0_f64, 0usize);
    let side = g.cell_mm * l.scale;
    for row in 0..g.rows {
        for col in 0..g.cols {
            let v = g.at(col, row).unwrap_or(0.0);
            if v <= 0.0 {
                continue;
            }
            max = max.max(v);
            if v >= TOO_DENSE {
                dense += 1;
            }
            let x = (g.origin_mm.0 + col as f64 * g.cell_mm - l.left) * l.scale;
            let y = (g.origin_mm.1 + row as f64 * g.cell_mm - l.top) * l.scale;
            c.rect(x, y, x + side, y + side, ramp(v), 0.9);
        }
    }
    Ok((max, dense))
}

const JUMP_RGB: [f64; 3] = [0.25, 0.25, 0.3];
const TRIM_RGB: [f64; 3] = [0.86, 0.12, 0.12];
const STOP_RGB: [f64; 3] = [0.95, 0.6, 0.0];
/// Dash and gap of a jump line, in pixels.
const DASH: f64 = 4.0;
const GAP: f64 = 3.0;

fn dashed(c: &mut Canvas, a: (f64, f64), b: (f64, f64)) {
    let len = (b.0 - a.0).hypot(b.1 - a.1);
    let period = DASH + GAP;
    // A very long jump (in an unprepared plan) draws solid rather than in countless dashes.
    if !len.is_finite() || len / period > 10_000.0 {
        c.capsule(a, b, 0.6, |_, _| (JUMP_RGB, 0.8));
        return;
    }
    let at = |d: f64| {
        let t = if len > 0.0 { (d / len).min(1.0) } else { 0.0 };
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    let mut d = 0.0;
    while d < len {
        c.capsule(at(d), at(d + DASH), 0.6, |_, _| (JUMP_RGB, 0.8));
        d += period;
    }
}

/// Jumps, needle points and function markers over the picture.
fn draw_overlays(c: &mut Canvas, plan: &StitchPlan, l: &Layout, o: &PreviewOptions) {
    let mark = (1.2 * l.scale).clamp(4.0, 24.0);
    let dot = (0.3 * o.thread_mm * l.scale).clamp(0.8, 4.0);
    let mut last: Option<Pt> = None;
    for (i, b) in plan.blocks.iter().enumerate() {
        if i > 0
            && o.show_functions
            && let Some(p) = last
        {
            let at = l.px(p);
            c.ring(at, mark * 1.4, 1.5, [0.1; 3], true);
            c.ring(at, mark * 1.4 - 1.5, 0.0, unit_rgb(b.thread.rgb), true);
        }
        for s in &b.steps {
            match *s {
                Step::Stitch(p) => {
                    if o.show_needles {
                        c.ring(l.px(p), dot + 0.7, 0.0, [0.1; 3], true);
                        c.ring(l.px(p), dot, 0.0, [1.0; 3], true);
                    }
                    last = Some(p);
                }
                Step::Jump(p) => {
                    if o.show_jumps
                        && let Some(q) = last
                    {
                        dashed(c, l.px(q), l.px(p));
                    }
                    last = Some(p);
                }
                Step::Trim => {
                    if o.show_functions
                        && let Some(p) = last
                    {
                        c.ring(l.px(p), mark, 2.0, TRIM_RGB, false);
                    }
                }
                Step::Stop => {
                    if o.show_functions
                        && let Some(p) = last
                    {
                        c.ring(l.px(p), mark * 1.8, 3.0, STOP_RGB, false);
                    }
                }
            }
        }
    }
}

/// Draw `plan` with `o`. Errors for bad options, a plan with no stitches or jumps, a picture larger
/// than [`MAX_SIDE`] on a side, or too little memory for it. Besides the picture it allocates only
/// the density grid, at most [`MAX_DENSITY_CELLS`] squared.
pub fn preview(plan: &StitchPlan, o: &PreviewOptions) -> Result<Preview, String> {
    let l = Layout::new(plan, o)?;
    let mut c = Canvas::new(l.width, l.height, o.fabric)?;
    let (max_coverage, dense_cells) = match o.mode {
        PreviewMode::Realistic => {
            draw_realistic(&mut c, plan, &l, o);
            (0.0, 0)
        }
        PreviewMode::Density => draw_density(&mut c, plan, &l, o)?,
    };
    draw_overlays(&mut c, plan, &l, o);
    Ok(Preview { width: l.width, height: l.height, rgba: c.px, max_coverage, dense_cells })
}

/// The plan up to and including its `stitches`-th sewn stitch (needle penetration): whole blocks
/// before it, and the block it falls in cut after it. Jumps, trims and stops after it are left
/// out, so what is kept is what a machine would have sewn at that moment. `stitches` of 0 gives a
/// plan with no blocks.
pub fn prefix_plan(plan: &StitchPlan, stitches: usize) -> StitchPlan {
    let mut left = stitches;
    let mut blocks = Vec::new();
    for b in &plan.blocks {
        if left == 0 {
            break;
        }
        let mut steps = Vec::new();
        for s in &b.steps {
            if left == 0 {
                break;
            }
            if matches!(s, Step::Stitch(_)) {
                left -= 1;
            }
            steps.push(*s);
        }
        blocks.push(crate::Block { thread: b.thread.clone(), steps });
    }
    StitchPlan { name: plan.name.clone(), blocks }
}

/// Draw the first `stitches` sewn stitches of `plan` (see [`prefix_plan`]) in the picture
/// [`preview`] makes of the whole plan: the same size and position, so frames of a playback line
/// up. The last needle point is marked with a ring. The layout comes from the whole plan, so a
/// plan with no stitches or jumps is an error as for [`preview`]; `stitches` of 0 gives the bare
/// fabric.
pub fn preview_prefix(plan: &StitchPlan, o: &PreviewOptions, stitches: usize) -> Result<Preview, String> {
    let l = Layout::new(plan, o)?;
    let part = prefix_plan(plan, stitches);
    let mut c = Canvas::new(l.width, l.height, o.fabric)?;
    let (max_coverage, dense_cells) = if part.blocks.is_empty() {
        (0.0, 0)
    } else {
        match o.mode {
            PreviewMode::Realistic => {
                draw_realistic(&mut c, &part, &l, o);
                (0.0, 0)
            }
            PreviewMode::Density => draw_density(&mut c, &part, &l, o)?,
        }
    };
    draw_overlays(&mut c, &part, &l, o);
    let needle = part.blocks.iter().rev().flat_map(|b| b.steps.iter().rev()).find_map(|s| match s {
        Step::Stitch(p) => Some(*p),
        _ => None,
    });
    if let Some(p) = needle {
        let at = l.px(p);
        let mark = (0.8 * l.scale).clamp(4.0, 14.0);
        c.ring(at, mark, 2.0, [0.1; 3], false);
        c.ring(at, mark, 1.0, [1.0, 0.85, 0.1], false);
    }
    Ok(Preview { width: l.width, height: l.height, rgba: c.px, max_coverage, dense_cells })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Block, Thread};

    fn plan(blocks: Vec<(Vec<Step>, [u8; 3])>) -> StitchPlan {
        StitchPlan {
            name: "t".into(),
            blocks: blocks.into_iter().map(|(steps, rgb)| Block { thread: Thread { rgb, ..Thread::default() }, steps }).collect(),
        }
    }

    fn st(x: i32, y: i32) -> Step {
        Step::Stitch(Pt { x, y })
    }

    /// Coverage per pixel of a black capsule on white.
    fn coverage(c: &Canvas) -> Vec<f64> {
        c.px.chunks(4).map(|p| 1.0 - f64::from(p[0]) / 255.0).collect()
    }

    #[test]
    fn a_capsule_is_symmetric_about_its_centre_line() {
        let mut c = Canvas::new(40, 21, [255; 3]).unwrap();
        c.capsule((10.0, 10.5), (30.0, 10.5), 3.0, |_, _| ([0.0; 3], 1.0));
        let cov = coverage(&c);
        for y in 0..21 {
            for x in 0..40 {
                let (a, b) = (cov[y * 40 + x], cov[(20 - y) * 40 + x]);
                assert!((a - b).abs() < 1.0 / 255.0 + 1e-9, "({x}, {y}): {a} vs {b}");
                let m = cov[y * 40 + (39 - x)];
                assert!((a - m).abs() < 1.0 / 255.0 + 1e-9, "mirror ({x}, {y}): {a} vs {m}");
            }
        }
    }

    #[test]
    fn a_stitch_covers_its_area() {
        let mut c = Canvas::new(80, 40, [255; 3]).unwrap();
        let (len, r) = (50.0, 2.0);
        c.capsule((15.0, 20.0), (15.0 + len, 20.0), r, |_, _| ([0.0; 3], 1.0));
        let total: f64 = coverage(&c).iter().sum();
        let area = 2.0 * r * len + std::f64::consts::PI * r * r;
        assert!((total - area).abs() / area < 0.03, "{total} vs {area}");
        // Diagonal too.
        let mut d = Canvas::new(80, 80, [255; 3]).unwrap();
        d.capsule((10.0, 10.0), (10.0 + len / 2f64.sqrt(), 10.0 + len / 2f64.sqrt()), r, |_, _| ([0.0; 3], 1.0));
        let total: f64 = coverage(&d).iter().sum();
        assert!((total - area).abs() / area < 0.03, "{total} vs {area}");
    }

    #[test]
    fn drawing_past_the_edges_stays_inside() {
        let mut c = Canvas::new(10, 10, [255; 3]).unwrap();
        c.capsule((-1e9, -50.0), (1e9, 60.0), 30.0, |_, _| ([0.0; 3], 1.0));
        c.capsule((f64::NAN, 0.0), (5.0, f64::INFINITY), 3.0, |_, _| ([0.0; 3], 1.0));
        c.ring((-5.0, 15.0), 8.0, 2.0, [0.0; 3], false);
        c.rect(-3.0, -3.0, 1e12, 4.0, [0.0; 3], 1.0);
        assert_eq!(c.px.len(), 400);
    }

    #[test]
    fn the_picture_is_the_design_plus_a_margin() {
        let p = plan(vec![(vec![st(0, 0), st(100, 0), st(100, 50)], [200, 0, 0])]);
        let o = PreviewOptions::default();
        // 10 mm × 5 mm plus 2 mm each side at 10 px/mm.
        assert_eq!(preview_size(&p, &o).unwrap(), (140, 90));
        let img = preview(&p, &o).unwrap();
        assert_eq!(img.rgba.len(), 140 * 90 * 4);
        // The fabric in the corner, red thread on the line.
        assert_eq!(&img.rgba[..4], &[236, 232, 222, 255]);
        let i = (20 * 140 + 70) * 4;
        assert!(img.rgba[i] > img.rgba[i + 1] + 60, "{:?}", &img.rgba[i..i + 4]);
    }

    #[test]
    fn bad_options_and_huge_designs_are_errors() {
        let p = plan(vec![(vec![st(0, 0), st(100_000, 0)], [0; 3])]);
        let err = preview(&p, &PreviewOptions::default()).unwrap_err();
        assert!(err.contains("more than 8192"), "{err}");
        for o in [
            PreviewOptions { scale: 0.0, ..PreviewOptions::default() },
            PreviewOptions { scale: f64::NAN, ..PreviewOptions::default() },
            PreviewOptions { thread_mm: 9.0, ..PreviewOptions::default() },
        ] {
            assert!(preview(&p, &o).is_err());
        }
        assert!(preview(&StitchPlan::default(), &PreviewOptions::default()).is_err());
        assert!(preview(&plan(vec![(vec![Step::Trim], [0; 3])]), &PreviewOptions::default()).is_err());
    }

    #[test]
    fn later_stitches_cover_earlier_ones() {
        let p = plan(vec![(vec![st(0, 0), st(100, 0)], [255, 0, 0]), (vec![st(50, -50), st(50, 50)], [0, 0, 255])]);
        let img = preview(&p, &PreviewOptions::default()).unwrap();
        let w = img.width as usize;
        // The crossing at (5 mm, 0) + margin: blue on top.
        let i = ((70 * w) + 70) * 4;
        assert!(img.rgba[i + 2] > img.rgba[i], "{:?}", &img.rgba[i..i + 4]);
    }

    #[test]
    fn jumps_needles_and_functions_draw_only_when_asked() {
        let p = plan(vec![
            (vec![st(0, 0), st(50, 0), Step::Trim, Step::Jump(Pt { x: 300, y: 0 }), st(300, 0), st(350, 0)], [0, 0, 0]),
            (vec![st(350, 100), st(300, 100), Step::Stop, st(250, 100)], [0, 0, 200]),
        ]);
        let plain = preview(&p, &PreviewOptions::default()).unwrap();
        let all = PreviewOptions { show_jumps: true, show_needles: true, show_functions: true, ..PreviewOptions::default() };
        let marked = preview(&p, &all).unwrap();
        let w = plain.width as usize;
        // Midway along the jump (17.5 mm, 0) + margin: fabric, then a dash or gap.
        let differs = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            plain.rgba[i..i + 3] != marked.rgba[i..i + 3]
        };
        assert!((150..250).any(|x| differs(x, 20)), "the jump shows");
        // The trim at (5 mm, 0): its ring of 12 px passes (70, 8).
        assert!(differs(70, 8), "the trim ring shows at the trim");
        for o in [
            PreviewOptions { show_jumps: true, ..PreviewOptions::default() },
            PreviewOptions { show_needles: true, ..PreviewOptions::default() },
            PreviewOptions { show_functions: true, ..PreviewOptions::default() },
        ] {
            assert_ne!(preview(&p, &o).unwrap().rgba, plain.rgba, "{o:?}");
        }
    }

    #[test]
    fn coincident_runs_double_the_density() {
        let run: Vec<Step> = (0..=20).map(|i| st(i * 5, 0)).collect();
        let once = density(&plan(vec![(run.clone(), [0; 3])]), 0.4).unwrap();
        let twice = density(&plan(vec![(run.clone(), [0; 3]), (run, [9; 3])]), 0.4).unwrap();
        let (a, b) = (once.at_mm(5.0, 0.0).unwrap(), twice.at_mm(5.0, 0.0).unwrap());
        // 1 mm of 0.4 mm thread in a 1 mm cell.
        assert!((a - 0.4).abs() < 1e-9, "{a}");
        assert!((b - 2.0 * a).abs() < 1e-9, "{b}");
        assert_eq!(once.at_mm(5.0, 3.0), None);
    }

    #[test]
    fn density_mode_counts_cells_that_are_too_dense() {
        // Six coincident 1 cm runs of 0.4 mm thread: coverage 2.4 per 1 mm cell.
        let run: Vec<Step> = (0..=10).map(|i| st(i * 10, 0)).collect();
        let p = plan((0..6).map(|_| (run.clone(), [0; 3])).collect());
        let img = preview(&p, &PreviewOptions { mode: PreviewMode::Density, ..PreviewOptions::default() }).unwrap();
        assert!((img.max_coverage - 2.4).abs() < 1e-6, "{}", img.max_coverage);
        // The ten cells the runs cross (the eleventh, past the last needle point, holds none).
        assert_eq!(img.dense_cells, 10);
        let one = plan(vec![(run, [0; 3])]);
        let light = preview(&one, &PreviewOptions { mode: PreviewMode::Density, ..PreviewOptions::default() }).unwrap();
        assert_eq!(light.dense_cells, 0);
        // The ramp runs green to red.
        assert!(ramp(TOO_DENSE)[0] > ramp(1.0)[0] && ramp(1.0)[1] > ramp(TOO_DENSE)[1]);
    }

    #[test]
    fn a_plan_of_two_million_steps_renders_within_the_picture() {
        // A 20 × 20 mm block of zig-zag stitches, revisited until the plan is full.
        let steps: Vec<Step> = (0..crate::MAX_STEPS).map(|i| st(((i * 7) % 200) as i32, ((i / 200) % 200) as i32)).collect();
        let p = plan(vec![(steps, [10, 120, 60])]);
        let o = PreviewOptions { scale: 1.0, ..PreviewOptions::default() };
        let img = preview(&p, &o).unwrap();
        assert_eq!((img.width, img.height), (24, 24));
        assert_eq!(img.rgba.len(), 24 * 24 * 4);
        let d = preview(&p, &PreviewOptions { mode: PreviewMode::Density, ..o }).unwrap();
        assert!(d.dense_cells > 0);
    }

    #[test]
    fn a_prefix_is_the_same_picture_with_fewer_stitches() {
        let p = plan(vec![(vec![st(0, 0), st(100, 0), st(100, 50)], [200, 0, 0]), (vec![st(0, 50), st(0, 100)], [0, 0, 200])]);
        let o = PreviewOptions::default();
        let whole = preview(&p, &o).unwrap();
        let none = preview_prefix(&p, &o, 0).unwrap();
        assert_eq!((none.width, none.height), (whole.width, whole.height));
        assert!(none.rgba.chunks(4).all(|px| px == [236, 232, 222, 255]), "no stitches, bare fabric");
        // The first two stitches sew the top edge only; the whole plan also has the right edge.
        let two = preview_prefix(&p, &o, 2).unwrap();
        let at = |img: &Preview, x: usize, y: usize| {
            let i = (y * img.width as usize + x) * 4;
            img.rgba[i]
        };
        assert_ne!(at(&two, 90, 20), 236, "the top edge is sewn");
        // Mid right edge (10 mm, 2.5 mm) + margin = (120, 45): thread in the whole, fabric in the prefix.
        assert_eq!(&two.rgba[(45 * two.width as usize + 120) * 4..][..3], &[236, 232, 222]);
        assert_ne!(&whole.rgba[(45 * whole.width as usize + 120) * 4..][..3], &[236, 232, 222]);
        // Past the end is the whole plan, with the needle marked at its end.
        let all = preview_prefix(&p, &o, 1000).unwrap();
        assert_ne!(all.rgba, whole.rgba);
        let cut = prefix_plan(&p, 4);
        assert_eq!((cut.blocks.len(), cut.blocks[1].steps.len()), (2, 1));
        assert_eq!(prefix_plan(&p, 3).blocks.len(), 1);
        assert!(preview_prefix(&StitchPlan::default(), &o, 3).is_err());
    }
}
