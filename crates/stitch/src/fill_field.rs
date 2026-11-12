//! Direction-following fill: a region sewn in rows of running stitch that bend with a direction
//! field (the way fur lies), instead of the straight parallel rows of [`crate::tatami`].
//!
//! Rows are evenly spaced streamlines of the field, placed as Jobard and Lefer describe
//! (*Creating evenly-spaced streamlines of arbitrary density*, 1997): a row is traced from a seed
//! both ways along the field (midpoint steps; a direction and its opposite are the same, so each
//! step keeps the sign of the last) until it leaves the region, turns too sharply, or comes within
//! half the spacing of a row already placed; new seeds are tried one spacing to either side of
//! every row, and a scan of the region seeds any part they don't reach. Each row is then cut into
//! stitches of random length within a range (seeded, so the same input sews the same), and with
//! feathering its ends are pulled in by a random amount, so the edge is ragged as fur is. Rows are
//! sewn nearest end first; the needle travels inside the region between them when the way is clear
//! and short, and is trimmed and jumps otherwise. The region tests are the tatami fill's.

use std::collections::VecDeque;

use vectorcraft_geom::FillRule;

use crate::fill::{ROUNDING_SLACK_MM, Region, frame, split_long, straight, to_plan};
use crate::{MAX_STEPS, Pt, TOO_MANY_STEPS};

type P = (f64, f64);

/// The most cells a [`DirectionGrid`] may have.
pub const MAX_GRID_CELLS: usize = 1 << 22;
/// The most streamline points one fill may trace.
const MAX_TRACED: usize = 4_000_000;
/// The most cells of the proximity index.
const MAX_INDEX_CELLS: usize = 1 << 24;
/// The error for a fill whose rows would be too many to trace.
pub const TOO_LARGE_FIELD_FILL: &str = "the direction fill is too large: use a larger spacing or fill a smaller shape";
/// The sharpest turn a row takes in one step before it stops, in degrees.
const MAX_TURN_DEG: f64 = 35.0;
/// How much of the fallback direction is blended in, so weak parts of the field follow it.
const FALLBACK_WEIGHT: f64 = 0.05;

/// A direction field on a grid in millimetres (page coordinates, y down): angle and weight per
/// cell, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectionGrid {
    /// The centre of cell `(0, 0)`, mm.
    pub origin: (f64, f64),
    /// A cell's side, mm.
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    /// Per cell: the direction, degrees counter-clockwise from horizontal as seen on the page (a
    /// direction and its opposite are the same).
    pub angles: Vec<f64>,
    /// Per cell: how much the direction counts, 0–1 (the field's coherence).
    pub weights: Vec<f64>,
}

impl DirectionGrid {
    /// One direction everywhere.
    pub fn uniform(angle: f64) -> Self {
        Self { origin: (0.0, 0.0), cell: 1.0, cols: 1, rows: 1, angles: vec![angle], weights: vec![1.0] }
    }

    fn check(&self) -> Result<(), String> {
        let n = self.cols.checked_mul(self.rows).filter(|&n| n > 0 && n <= MAX_GRID_CELLS);
        if n.is_none() || n != Some(self.angles.len()) || n != Some(self.weights.len()) {
            return Err("the direction field's grid doesn't match its size".into());
        }
        if !(self.cell.is_finite() && self.cell > 0.0 && self.origin.0.is_finite() && self.origin.1.is_finite()) {
            return Err("the direction field's cells must be finite and positive".into());
        }
        Ok(())
    }

    /// The doubled-angle vector at `p` (page y down: the angle's vector is `(cos a, −sin a)`),
    /// cells blended bilinearly by weight.
    fn doubled(&self, p: P) -> P {
        let fx = ((p.0 - self.origin.0) / self.cell).clamp(0.0, (self.cols - 1) as f64);
        let fy = ((p.1 - self.origin.1) / self.cell).clamp(0.0, (self.rows - 1) as f64);
        if !(fx.is_finite() && fy.is_finite()) {
            return (0.0, 0.0);
        }
        let (c0, r0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - c0 as f64, fy - r0 as f64);
        let mut v = (0.0, 0.0);
        for (dc, dr, w) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let i = (r0 + dr).min(self.rows - 1) * self.cols + (c0 + dc).min(self.cols - 1);
            let (Some(a), Some(wt)) = (self.angles.get(i), self.weights.get(i)) else { continue };
            if !(a.is_finite() && wt.is_finite()) {
                continue;
            }
            // y down: the page angle `a` is the image angle `−a`.
            let t = (-2.0 * a).to_radians();
            let k = w * wt.clamp(0.0, 1.0);
            v.0 += k * t.cos();
            v.1 += k * t.sin();
        }
        v
    }

    /// The unit direction at `p` with `fallback` (a doubled-angle unit vector) blended in a little;
    /// `fallback` itself where the field cancels out.
    fn direction(&self, p: P, fallback: P) -> P {
        let v = self.doubled(p);
        let v = (v.0 + FALLBACK_WEIGHT * fallback.0, v.1 + FALLBACK_WEIGHT * fallback.1);
        let s = v.0.hypot(v.1);
        let t = if s < 1e-6 { fallback.1.atan2(fallback.0) / 2.0 } else { v.1.atan2(v.0) / 2.0 };
        (t.cos(), t.sin())
    }

    /// The weighted mean direction over the cells whose centres lie inside `rings` by `rule`, in
    /// degrees `[0, 180)`: the region's dominant direction. `None` when no cell centre is inside or
    /// they have no direction.
    pub fn mean_angle(&self, rings: &[Vec<(f64, f64)>], rule: FillRule) -> Option<f64> {
        self.check().ok()?;
        let region = Region::new(rings, rule, frame(0.0), self.cell.max(0.05)).ok()??;
        let mut v = (0.0, 0.0);
        for r in 0..self.rows {
            for c in 0..self.cols {
                let p = (self.origin.0 + c as f64 * self.cell, self.origin.1 + r as f64 * self.cell);
                if !region.inside(p) {
                    continue;
                }
                let i = r * self.cols + c;
                let (Some(a), Some(w)) = (self.angles.get(i), self.weights.get(i)) else { continue };
                if a.is_finite() && w.is_finite() {
                    let t = (2.0 * a).to_radians();
                    v.0 += w * t.cos();
                    v.1 += w * t.sin();
                }
            }
        }
        if v.0.hypot(v.1) < 1e-9 {
            return None;
        }
        Some((v.1.atan2(v.0).to_degrees() / 2.0).rem_euclid(180.0))
    }
}

/// Which ends of each row are feathered: `Start` and `End` are the row's ends going along the
/// fallback direction (its angle as given, not the opposite).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FeatherEdges {
    #[default]
    Both,
    Start,
    End,
}

/// Direction fill settings. Lengths in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldFillParams {
    /// Distance between rows.
    pub spacing: f64,
    /// The stitch lengths along a row, each drawn at random between these.
    pub min_length: f64,
    pub max_length: f64,
    /// The most a feathered row end is pulled in (0: no feathering).
    pub feather: f64,
    pub edges: FeatherEdges,
    /// The seed of the random lengths: the same seed sews the same.
    pub seed: u64,
    /// The direction (degrees, as [`crate::FillParams::angle`]) where the field has none, and
    /// which way rows run for [`FeatherEdges`].
    pub fallback_angle: f64,
    /// The longest travel inside the region from one row to the next; longer is a trim and a jump.
    pub max_travel: f64,
    /// The longest stitch the machine takes.
    pub max_stitch: f64,
}

impl Default for FieldFillParams {
    fn default() -> Self {
        Self {
            spacing: 0.4,
            min_length: 2.0,
            max_length: 4.0,
            feather: 0.0,
            edges: FeatherEdges::Both,
            seed: 1,
            fallback_angle: 0.0,
            max_travel: 5.0,
            max_stitch: 12.1,
        }
    }
}

/// SplitMix64: a small, fast, seeded generator, the same on every platform.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[lo, hi]`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        let u = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        lo + (hi - lo) * u
    }
}

fn dist(a: P, b: P) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The points of the rows placed so far, by cell, to find the rows near a point.
struct Index {
    min: P,
    cell: f64,
    w: usize,
    h: usize,
    cells: Vec<Vec<(f32, f32, u32, i32)>>,
}

impl Index {
    fn new(min: P, max: P, cell: f64) -> Result<Self, String> {
        let w = ((max.0 - min.0) / cell).ceil() + 3.0;
        let h = ((max.1 - min.1) / cell).ceil() + 3.0;
        if !(w.is_finite() && h.is_finite()) || w * h > MAX_INDEX_CELLS as f64 {
            return Err(TOO_LARGE_FIELD_FILL.into());
        }
        let (w, h) = (w as usize, h as usize);
        Ok(Self { min: (min.0 - cell, min.1 - cell), cell, w, h, cells: vec![vec![]; w * h] })
    }

    fn at(&self, p: P) -> Option<(usize, usize)> {
        let x = ((p.0 - self.min.0) / self.cell).floor();
        let y = ((p.1 - self.min.1) / self.cell).floor();
        (x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h).then_some((x as usize, y as usize))
    }

    fn add(&mut self, p: P, line: u32, idx: i32) {
        if let Some((x, y)) = self.at(p)
            && let Some(c) = self.cells.get_mut(y * self.w + x)
        {
            c.push((p.0 as f32, p.1 as f32, line, idx));
        }
    }

    /// Whether a point lies within `d` (at most the cell size) of `p`, leaving out the points of
    /// row `line` within `skip` steps of `idx`.
    fn near(&self, p: P, d: f64, line: u32, idx: i32, skip: i32) -> bool {
        let Some((x, y)) = self.at(p) else { return false };
        for yy in y.saturating_sub(1)..=(y + 1).min(self.h - 1) {
            for xx in x.saturating_sub(1)..=(x + 1).min(self.w - 1) {
                let Some(c) = self.cells.get(yy * self.w + xx) else { continue };
                for &(qx, qy, l, i) in c {
                    if l == line && (i - idx).abs() <= skip {
                        continue;
                    }
                    if dist(p, (f64::from(qx), f64::from(qy))) < d {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Direction-following fill of the region the closed `rings` (millimetres, page coordinates)
/// enclose by `rule`, its rows following `grid`. Returns runs of needle points in sewing order; the
/// plan joins them with a trim and a jump. More than `budget` points is an error.
pub fn field_fill(
    rings: &[Vec<(f64, f64)>],
    rule: FillRule,
    grid: &DirectionGrid,
    p: &FieldFillParams,
    budget: usize,
) -> Result<Vec<Vec<Pt>>, String> {
    let budget = budget.min(MAX_STEPS);
    grid.check()?;
    let ok = |v: f64, lo: f64| v.is_finite() && v >= lo;
    if !(ok(p.spacing, 0.1)
        && ok(p.min_length, 0.3)
        && ok(p.max_length, p.min_length)
        && ok(p.feather, 0.0)
        && ok(p.max_travel, 0.0)
        && ok(p.max_stitch, 0.5)
        && p.fallback_angle.is_finite())
    {
        return Err("direction fill settings must be finite and positive, the longest stitch no shorter than the shortest".into());
    }
    let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    for q in rings.iter().flatten().filter(|q| q.0.is_finite() && q.1.is_finite()) {
        lo = (lo.0.min(q.0), lo.1.min(q.1));
        hi = (hi.0.max(q.0), hi.1.max(q.1));
    }
    if !(lo.0.is_finite() && hi.0.is_finite()) {
        return Ok(vec![]);
    }
    let band = p.spacing.max((hi.1 - lo.1) / 50_000.0);
    let Some(region) = Region::new(rings, rule, frame(0.0), band)? else { return Ok(vec![]) };
    let lines = streamlines(&region, grid, p, lo, hi)?;
    let rows = stitch_rows(&lines, p);
    let runs = sew(&region, rows, p, budget)?;
    let max = (p.max_stitch - ROUNDING_SLACK_MM).max(0.3);
    to_plan(runs.into_iter().map(|r| split_long(&r, max)).collect(), budget)
}

/// The evenly spaced streamlines of `grid` inside `region` (bounds `lo`–`hi`), each running along
/// the fallback direction.
fn streamlines(region: &Region, grid: &DirectionGrid, p: &FieldFillParams, lo: P, hi: P) -> Result<Vec<Vec<P>>, String> {
    let sep = p.spacing;
    let test = sep * 0.5;
    let h = (sep * 0.5).clamp(0.05, 0.5);
    let skip = (2.0 * test / h).ceil() as i32 + 2;
    let max_steps = ((hi.0 - lo.0).hypot(hi.1 - lo.1) * 3.0 / h).ceil().min(1e6) as usize + 8;
    let a = (-2.0 * p.fallback_angle).to_radians();
    let fallback = (a.cos(), a.sin());
    let reference = {
        let r = p.fallback_angle.to_radians();
        (r.cos(), -r.sin())
    };
    let cos_turn = MAX_TURN_DEG.to_radians().cos();
    let mut index = Index::new(lo, hi, sep)?;
    let mut lines: Vec<Vec<P>> = vec![];
    let mut traced = 0usize;
    let mut queue: VecDeque<P> = VecDeque::new();
    // The scan for seeds the queue doesn't reach: a grid at the spacing.
    let (sw, sh) = (((hi.0 - lo.0) / sep).ceil() as usize + 1, ((hi.1 - lo.1) / sep).ceil() as usize + 1);
    if sw.saturating_mul(sh) > MAX_INDEX_CELLS {
        return Err(TOO_LARGE_FIELD_FILL.into());
    }
    let mut scan = 0usize;
    let aligned = |d: P, prev: P| if d.0 * prev.0 + d.1 * prev.1 < 0.0 { (-d.0, -d.1) } else { d };
    loop {
        let seed = match queue.pop_front() {
            Some(s) => s,
            None => {
                let mut found = None;
                while scan < sw * sh {
                    let (i, j) = (scan % sw, scan / sw);
                    scan += 1;
                    // Half a cell in, so a seed isn't on the outline of a rectangle.
                    let q = (lo.0 + (i as f64 + 0.5) * sep, lo.1 + (j as f64 + 0.5) * sep);
                    if region.inside(q) && !index.near(q, sep * 0.75, u32::MAX, 0, 0) {
                        found = Some(q);
                        break;
                    }
                }
                match found {
                    Some(q) => q,
                    None => break,
                }
            }
        };
        if !region.inside(seed) || index.near(seed, sep * 0.75, u32::MAX, 0, 0) {
            continue;
        }
        let id = u32::try_from(lines.len()).map_err(|_| TOO_LARGE_FIELD_FILL.to_string())?;
        let d0 = aligned(grid.direction(seed, fallback), reference);
        index.add(seed, id, 0);
        let mut halves: [Vec<P>; 2] = [vec![], vec![]];
        for (side, sign) in [(0usize, 1.0), (1usize, -1.0)] {
            let mut prev = (d0.0 * sign, d0.1 * sign);
            let mut at = seed;
            for step in 1..=max_steps {
                let idx = if side == 0 { step as i32 } else { -(step as i32) };
                let d1 = aligned(grid.direction(at, fallback), prev);
                let mid = (at.0 + d1.0 * h * 0.5, at.1 + d1.1 * h * 0.5);
                let d2 = aligned(grid.direction(mid, fallback), d1);
                if d2.0 * prev.0 + d2.1 * prev.1 < cos_turn {
                    break;
                }
                let q = (at.0 + d2.0 * h, at.1 + d2.1 * h);
                if !region.inside(q) {
                    // The outline, between the last point inside and the first outside.
                    let (mut a, mut b) = (at, q);
                    for _ in 0..12 {
                        let m = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
                        if region.inside(m) { a = m } else { b = m }
                    }
                    if dist(a, at) > 1e-6 && !index.near(a, test, id, idx, skip) {
                        index.add(a, id, idx);
                        if let Some(half) = halves.get_mut(side) {
                            half.push(a);
                        }
                    }
                    break;
                }
                if index.near(q, test, id, idx, skip) {
                    break;
                }
                index.add(q, id, idx);
                if let Some(half) = halves.get_mut(side) {
                    half.push(q);
                }
                traced += 1;
                if traced > MAX_TRACED {
                    return Err(TOO_LARGE_FIELD_FILL.into());
                }
                prev = d2;
                at = q;
            }
        }
        let [fwd, back] = halves;
        let mut line: Vec<P> = back.into_iter().rev().collect();
        line.push(seed);
        line.extend(fwd);
        // New seeds one spacing to either side, about every spacing along the row.
        let every = ((sep / h).round() as usize).max(1);
        for w in line.windows(2).step_by(every) {
            if let [a, b] = w {
                let l = dist(*a, *b);
                if l < 1e-9 {
                    continue;
                }
                let n = (-(b.1 - a.1) / l * sep, (b.0 - a.0) / l * sep);
                queue.push_back((a.0 + n.0, a.1 + n.1));
                queue.push_back((a.0 - n.0, a.1 - n.1));
            }
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Each line cut into stitches (the needle points of the row, start first), feathered ends pulled
/// in, lengths drawn at random within the range. Lines too short for one stitch are dropped.
fn stitch_rows(lines: &[Vec<P>], p: &FieldFillParams) -> Vec<Vec<P>> {
    let mut rng = Rng(p.seed);
    let (min, max) = (p.min_length, p.max_length);
    let mut rows = vec![];
    for line in lines {
        // Cumulative length along the line.
        let mut cum = Vec::with_capacity(line.len());
        let mut total = 0.0;
        cum.push(0.0);
        for w in line.windows(2) {
            if let [a, b] = w {
                total += dist(*a, *b);
            }
            cum.push(total);
        }
        let feather = |rng: &mut Rng, on: bool| if on && p.feather > 0.0 { rng.range(0.0, p.feather) } else { 0.0 };
        let mut cut0 = feather(&mut rng, matches!(p.edges, FeatherEdges::Both | FeatherEdges::Start));
        let mut cut1 = feather(&mut rng, matches!(p.edges, FeatherEdges::Both | FeatherEdges::End));
        if total < min {
            continue;
        }
        // Feathering never takes a row below one stitch.
        let room = total - min;
        if cut0 + cut1 > room {
            let k = room / (cut0 + cut1);
            cut0 *= k;
            cut1 *= k;
        }
        let mut len = total - cut0 - cut1;
        let n_min = (len / max).ceil().max(1.0);
        let n_max = (len / min).floor().max(1.0);
        let n = if n_min > n_max {
            // No whole number of stitches fits: shorten the row at its end.
            len = n_max * max;
            n_max
        } else {
            (len / ((min + max) * 0.5)).round().clamp(n_min, n_max)
        };
        let n = n as usize;
        let mut lengths = vec![len / n as f64; n];
        for i in 0..n.saturating_sub(1) {
            let (Some(&a), Some(&b)) = (lengths.get(i), lengths.get(i + 1)) else { continue };
            let lo = -(a - min).min(max - b).max(0.0);
            let hi = (max - a).min(b - min).max(0.0);
            let d = rng.range(lo, hi);
            if let Some([x, y]) = lengths.get_mut(i..i + 2) {
                *x = a + d;
                *y = b - d;
            }
        }
        // The needle points at those distances along the line.
        let at = |s: f64| -> P {
            let i = cum.partition_point(|&c| c < s).clamp(1, cum.len().max(2) - 1);
            let (Some(&c0), Some(&c1), Some(&a), Some(&b)) = (cum.get(i - 1), cum.get(i), line.get(i - 1), line.get(i)) else {
                return line.last().copied().unwrap_or((0.0, 0.0));
            };
            let t = if c1 - c0 > 1e-12 { ((s - c0) / (c1 - c0)).clamp(0.0, 1.0) } else { 0.0 };
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        };
        let mut s = cut0;
        let mut row = vec![at(s)];
        for l in lengths {
            s += l;
            row.push(at(s));
        }
        rows.push(row);
    }
    rows
}

/// The rows in sewing order, nearest end first, as runs: joined by travel inside the region when
/// it is clear and short, split otherwise.
fn sew(region: &Region, rows: Vec<Vec<P>>, p: &FieldFillParams, budget: usize) -> Result<Vec<Vec<P>>, String> {
    let ends: Vec<(P, P)> = rows.iter().filter_map(|r| Some((*r.first()?, *r.last()?))).collect();
    let mut done = vec![false; rows.len()];
    // The row ends by cell, for the nearest-end search.
    let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    for &(a, b) in &ends {
        for q in [a, b] {
            lo = (lo.0.min(q.0), lo.1.min(q.1));
            hi = (hi.0.max(q.0), hi.1.max(q.1));
        }
    }
    let mut runs: Vec<Vec<P>> = vec![];
    if ends.is_empty() {
        return Ok(runs);
    }
    let cell = (p.max_length * 2.0).max(p.spacing * 4.0);
    let (gw, gh) = (((hi.0 - lo.0) / cell).floor() as usize + 1, ((hi.1 - lo.1) / cell).floor() as usize + 1);
    if gw.saturating_mul(gh) > MAX_INDEX_CELLS {
        return Err(TOO_LARGE_FIELD_FILL.into());
    }
    let key =
        |q: P| (((q.0 - lo.0) / cell).floor().max(0.0) as usize).min(gw - 1) + (((q.1 - lo.1) / cell).floor().max(0.0) as usize).min(gh - 1) * gw;
    let mut cells: Vec<Vec<usize>> = vec![vec![]; gw * gh];
    for (i, &(a, b)) in ends.iter().enumerate() {
        for k in [key(a), key(b)] {
            if let Some(c) = cells.get_mut(k)
                && c.last() != Some(&i)
            {
                c.push(i);
            }
        }
    }
    let nearest = |at: P, done: &[bool], cells: &mut Vec<Vec<usize>>| -> Option<(usize, bool)> {
        let (cx, cy) = (((at.0 - lo.0) / cell).floor().max(0.0) as usize, ((at.1 - lo.1) / cell).floor().max(0.0) as usize);
        let mut best: Option<(f64, usize, bool)> = None;
        for r in 0..gw.max(gh) + 1 {
            // Every end in ring r is at least (r − 1) cells away.
            if let Some((d, _, _)) = best
                && d < (r as f64 - 1.0) * cell
            {
                break;
            }
            let (x0, x1) = (cx.saturating_sub(r), (cx + r).min(gw - 1));
            let (y0, y1) = (cy.saturating_sub(r), (cy + r).min(gh - 1));
            for y in y0..=y1 {
                // The ring's cells only: whole rows at its top and bottom, its two sides between.
                let xs: Vec<usize> = if y.abs_diff(cy) == r {
                    (x0..=x1).collect()
                } else {
                    [cx.checked_sub(r), Some(cx + r).filter(|&x| x < gw && r > 0)].into_iter().flatten().collect()
                };
                for x in xs {
                    let Some(c) = cells.get_mut(y * gw + x) else { continue };
                    c.retain(|&i| !done.get(i).copied().unwrap_or(true));
                    for &i in c.iter() {
                        let Some(&(a, b)) = ends.get(i) else { continue };
                        for (q, fwd) in [(a, true), (b, false)] {
                            let d = dist(at, q);
                            if best.is_none_or(|(bd, _, _)| d < bd) {
                                best = Some((d, i, fwd));
                            }
                        }
                    }
                }
            }
        }
        best.map(|(_, i, f)| (i, f))
    };
    let mut run: Vec<P> = vec![];
    let mut points = 0usize;
    let mut at = ends.first().map(|e| e.0).unwrap_or((0.0, 0.0));
    for _ in 0..rows.len() {
        let Some((i, fwd)) = nearest(at, &done, &mut cells) else { break };
        let Some(d) = done.get_mut(i) else { break };
        *d = true;
        let Some(row) = rows.get(i) else { continue };
        let start = if fwd { row.first() } else { row.last() }.copied().unwrap_or(at);
        if !run.is_empty() {
            if dist(at, start) <= p.max_travel && region.clear(at, start) {
                straight(&mut run, at, start, p.max_length);
            } else {
                points = points.saturating_add(run.len());
                runs.push(std::mem::take(&mut run));
            }
        }
        if fwd {
            run.extend(row.iter().copied().skip(usize::from(!run.is_empty())));
        } else {
            run.extend(row.iter().rev().copied().skip(usize::from(!run.is_empty())));
        }
        at = run.last().copied().unwrap_or(at);
        if points.saturating_add(run.len()) > budget.saturating_mul(2) {
            return Err(TOO_MANY_STEPS.into());
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<P> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    fn stitches(runs: &[Vec<Pt>]) -> Vec<(P, P)> {
        runs.iter().flat_map(|r| r.windows(2).map(|w| (w[0].to_mm(), w[1].to_mm()))).collect()
    }

    /// The length-weighted mean direction of `segs`, degrees `[0, 180)`.
    fn mean_dir(segs: &[(P, P)]) -> f64 {
        let mut v = (0.0, 0.0);
        for &(a, b) in segs {
            let l = dist(a, b);
            let t = 2.0 * (-(b.1 - a.1)).atan2(b.0 - a.0);
            v.0 += l * t.cos();
            v.1 += l * t.sin();
        }
        (v.1.atan2(v.0).to_degrees() / 2.0).rem_euclid(180.0)
    }

    fn angle_error(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(180.0);
        d.min(180.0 - d)
    }

    /// A field whose direction turns with x: `a0` at the left, `a0 + turn` at the right.
    fn turning(w: f64, h: f64, a0: f64, turn: f64) -> DirectionGrid {
        let (cols, rows) = (12, 8);
        let mut angles = vec![];
        for _ in 0..rows {
            for c in 0..cols {
                angles.push(a0 + turn * c as f64 / (cols - 1) as f64);
            }
        }
        DirectionGrid {
            origin: (0.0, 0.0),
            cell: (w / (cols - 1) as f64).max(h / (rows - 1) as f64),
            cols,
            rows,
            angles,
            weights: vec![1.0; cols * rows],
        }
    }

    #[test]
    fn a_uniform_field_sews_straight_rows_at_its_angle() {
        for angle in [0.0, 30.0, 90.0, 135.0] {
            let runs =
                field_fill(&[rect(0.0, 0.0, 30.0, 20.0)], FillRule::NonZero, &DirectionGrid::uniform(angle), &FieldFillParams::default(), MAX_STEPS)
                    .unwrap();
            let segs = stitches(&runs);
            assert!(segs.len() > 100);
            assert!(angle_error(mean_dir(&segs), angle) < 2.0, "{angle}: {}", mean_dir(&segs));
            // Covered: most of the area lies within a spacing of a needle point.
            let pts: Vec<P> = runs.iter().flatten().map(|p| p.to_mm()).collect();
            let mut covered = 0;
            for i in 0..30 {
                for j in 0..20 {
                    let q = (i as f64 + 0.5, j as f64 + 0.5);
                    if segs.iter().any(|&(a, b)| crate::fill::seg_dist(q, a, b) < 0.45) {
                        covered += 1;
                    }
                }
            }
            assert!(covered >= 570, "{angle}: {covered} of 600 covered");
            assert!(!pts.is_empty());
        }
    }

    #[test]
    fn rows_bend_with_the_field() {
        let grid = turning(40.0, 20.0, 0.0, 60.0);
        let runs = field_fill(&[rect(0.0, 0.0, 40.0, 20.0)], FillRule::NonZero, &grid, &FieldFillParams::default(), MAX_STEPS).unwrap();
        let segs = stitches(&runs);
        let left: Vec<_> = segs.iter().copied().filter(|(a, b)| a.0 < 8.0 && b.0 < 8.0).collect();
        let right: Vec<_> = segs.iter().copied().filter(|(a, b)| a.0 > 32.0 && b.0 > 32.0).collect();
        assert!(angle_error(mean_dir(&left), 5.0) < 6.0, "{}", mean_dir(&left));
        assert!(angle_error(mean_dir(&right), 55.0) < 6.0, "{}", mean_dir(&right));
    }

    #[test]
    fn the_same_seed_sews_the_same() {
        let p = FieldFillParams { feather: 1.5, ..Default::default() };
        let rings = [rect(0.0, 0.0, 20.0, 20.0)];
        let a = field_fill(&rings, FillRule::NonZero, &DirectionGrid::uniform(20.0), &p, MAX_STEPS).unwrap();
        let b = field_fill(&rings, FillRule::NonZero, &DirectionGrid::uniform(20.0), &p, MAX_STEPS).unwrap();
        assert_eq!(a, b);
        let c = field_fill(&rings, FillRule::NonZero, &DirectionGrid::uniform(20.0), &FieldFillParams { seed: 9, ..p }, MAX_STEPS).unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn feathered_ends_are_ragged() {
        // Rows run across x; with feathering their left ends stop at different places.
        let rings = [rect(0.0, 0.0, 30.0, 10.0)];
        let starts = |p: &FieldFillParams| -> Vec<f64> {
            let runs = field_fill(&rings, FillRule::NonZero, &DirectionGrid::uniform(0.0), p, MAX_STEPS).unwrap();
            runs.iter().flatten().map(|q| q.to_mm().0).filter(|&x| x < 5.0).collect()
        };
        let plain = starts(&FieldFillParams::default());
        let feathered = starts(&FieldFillParams { feather: 3.0, edges: FeatherEdges::Start, ..Default::default() });
        let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(min(&plain) < 0.1);
        // Feathered at the start: many rows begin well inside the edge.
        let inset = feathered.iter().filter(|&&x| x > 0.5 && x < 3.1).count();
        assert!(inset > 10, "{inset}");
    }

    #[test]
    fn bad_settings_and_grids_are_errors() {
        let rings = [rect(0.0, 0.0, 10.0, 10.0)];
        let g = DirectionGrid::uniform(0.0);
        let bad = [
            FieldFillParams { spacing: 0.0, ..Default::default() },
            FieldFillParams { min_length: 5.0, max_length: 4.0, ..Default::default() },
            FieldFillParams { feather: f64::NAN, ..Default::default() },
        ];
        for p in bad {
            assert!(field_fill(&rings, FillRule::NonZero, &g, &p, MAX_STEPS).is_err(), "{p:?}");
        }
        let broken = DirectionGrid { cols: 3, ..DirectionGrid::uniform(0.0) };
        assert!(field_fill(&rings, FillRule::NonZero, &broken, &FieldFillParams::default(), MAX_STEPS).is_err());
        assert_eq!(field_fill(&rings, FillRule::NonZero, &g, &FieldFillParams::default(), 50).unwrap_err(), TOO_MANY_STEPS);
        assert!(field_fill(&[rect(0.0, 0.0, 10_000.0, 10_000.0)], FillRule::NonZero, &g, &FieldFillParams::default(), MAX_STEPS).is_err());
        assert!(field_fill(&[vec![(0.0, 0.0), (1.0, 1.0)]], FillRule::NonZero, &g, &FieldFillParams::default(), MAX_STEPS).unwrap().is_empty());
    }

    #[test]
    fn mean_angle_reads_the_cells_inside() {
        let grid = turning(40.0, 20.0, 10.0, 0.0);
        assert!(angle_error(grid.mean_angle(&[rect(0.0, 0.0, 40.0, 20.0)], FillRule::NonZero).unwrap(), 10.0) < 1e-6);
        assert_eq!(grid.mean_angle(&[rect(500.0, 500.0, 501.0, 501.0)], FillRule::NonZero), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Along a row, every stitch is within the length range, feathered or not.
        #[test]
        fn row_stitches_keep_the_length_range(
            len in 0.5f64..60.0, min in 0.5f64..4.0, extra in 0.0f64..4.0, feather in 0.0f64..5.0, seed in 0u64..1000,
        ) {
            let max = min + extra;
            let line: Vec<P> = (0..=200).map(|i| (len * i as f64 / 200.0, 3.0)).collect();
            let p = FieldFillParams { min_length: min, max_length: max, feather, seed, ..Default::default() };
            let rows = stitch_rows(&[line], &p);
            if len < min {
                prop_assert!(rows.is_empty());
            } else {
                prop_assert_eq!(rows.len(), 1);
                for w in rows[0].windows(2) {
                    let l = dist(w[0], w[1]);
                    prop_assert!(l >= min - 1e-6 && l <= max + 1e-6, "{} not in {}–{}", l, min, max);
                }
                prop_assert!(rows[0].iter().all(|q| q.0 >= -1e-9 && q.0 <= len + 1e-9));
            }
        }

        /// A ring (a rectangle with a rectangular hole) in a turning field: every needle point
        /// inside the shape grown by a tolerance, no row stitch in the hole, every row stitch within
        /// the length range (rounded to plan units), no stitch past the maximum, bounded output.
        #[test]
        fn field_fill_stays_in_the_shape(
            w in 8.0f64..40.0, h in 8.0f64..40.0, hole in 0.1f64..0.6,
            a0 in -90.0f64..90.0, turn in -40.0f64..40.0, spacing in 0.3f64..1.2,
            min in 1.0f64..3.0, extra in 0.0f64..3.0, feather in 0.0f64..3.0, seed in 0u64..1000,
        ) {
            let (hx0, hy0) = (w * (0.5 - hole / 2.0), h * (0.5 - hole / 2.0));
            let (hx1, hy1) = (w - hx0, h - hy0);
            let rings = [rect(0.0, 0.0, w, h), rect(hx0, hy0, hx1, hy1)];
            let max = min + extra;
            let p = FieldFillParams { spacing, min_length: min, max_length: max, feather, seed, fallback_angle: a0, ..Default::default() };
            let budget = 200_000;
            let runs = field_fill(&rings, FillRule::EvenOdd, &turning(w, h, a0, turn), &p, budget).unwrap();
            prop_assert!(!runs.is_empty());
            prop_assert!(runs.iter().map(Vec::len).sum::<usize>() <= budget);
            let tol = 0.15;
            for q in runs.iter().flatten().map(|p| p.to_mm()) {
                prop_assert!(q.0 > -tol && q.0 < w + tol && q.1 > -tol && q.1 < h + tol, "{:?}", q);
                prop_assert!(!(q.0 > hx0 + tol && q.0 < hx1 - tol && q.1 > hy0 + tol && q.1 < hy1 - tol), "{:?} in the hole", q);
            }
            let segs = stitches(&runs);
            for &(a, b) in &segs {
                let l = dist(a, b);
                prop_assert!(l <= max + 0.15, "{} > {}", l, max);
                // Row stitches and travel never cross the hole.
                let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                prop_assert!(!(m.0 > hx0 + tol && m.0 < hx1 - tol && m.1 > hy0 + tol && m.1 < hy1 - tol), "{:?}–{:?} crosses the hole", a, b);
            }
            // Stitches shorter than the range are the travel from one row's end to the next row's
            // start (row stitches keep the range: `row_stitches_keep_the_length_range`), at most one
            // per row, and no longer than the travel allowed.
            let short: Vec<f64> = segs.iter().map(|&(a, b)| dist(a, b)).filter(|&l| l < min * 0.93 - 0.15).collect();
            prop_assert!(short.len() * 2 <= segs.len(), "{} of {} stitches shorter than {}", short.len(), segs.len(), min);
            prop_assert!(short.iter().all(|&l| l <= p.max_travel + 0.15));
        }
    }
}
