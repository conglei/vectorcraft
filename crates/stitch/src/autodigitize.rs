//! Auto-digitizing (E10): what stitch objects a shape becomes, by its width.
//!
//! A closed outline (mm, holes included, nonzero rule) is reduced to its skeleton and rails as
//! lettering does ([`crate::columns`]). Each stretch is then classed by its width as drawn:
//! - narrower than [`AutoParams::min_column`] (1 mm): running stitch along the centre line;
//! - up to [`AutoParams::max_satin`] (8 mm): a satin column between the rails;
//! - wider: tatami fill. The wide part is the shape's morphological opening by a disc as wide as
//!   the widest satin (Serra, *Image Analysis and Mathematical Morphology*, 1982): every place a
//!   disc of that diameter fits. What is left over is sewn as columns and runs where it makes a
//!   stretch at least [`STUB_FACTOR`] times the widest satin long (a tail, an arm); shorter
//!   leftovers (the corners of a square, a fillet) are filled with the wide part.
//!
//! Shape-level rules: a compact blob (its centre lines shorter than [`COMPACT`] times its width,
//! at least half the widest satin wide: a disc, a fat square) is filled, not sewn as a stubby
//! column; an outline with more strokes than the skeleton graph holds is filled. Open stroked
//! paths are classed by their stroke weight ([`stroke_kind`]).
//!
//! The thresholds come from public digitizing guides:
//! - areas under about 1 mm wide are running stitch (<https://graphics-pro.com/?p=95539>); satin is
//!   not narrower than about 1 mm
//!   (<https://hoopingstation.com/blogs/articles/run-satin-and-fill-the-only-3-stitch-types-you-need-to-digitize-and-stitch-almost-anything>);
//! - satin is kept to about 7–10 mm before it snags or becomes fill
//!   (<https://wilcom.com/resources/blog/embroidery-stitch-techniques>,
//!   <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Large_Lettering.htm>,
//!   <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Overview_of_Object_Types.htm>);
//!   8 mm, as lettering uses, is the default;
//! - a stroke becomes a satin column as wide as its stroke weight (Ink/Stitch's Stroke to Satin,
//!   documentation only: <https://inkstitch.org/docs/satin-tools/>).
//!
//! [`STUB_FACTOR`], [`COMPACT`] and the overlap between a column and the fill it meets are
//! estimated, to check at the stitch-out.

use std::collections::HashMap;

use crate::columns::{self, Stroke, dist, pieces};
use crate::lettering::{Column, MAX_SATIN_MM, MIN_COLUMN_MM, P};
use crate::skeleton::{self, Grid};

/// A leftover stretch shorter than this many widest-satin widths is filled with the wide part it
/// hangs off (estimated).
pub const STUB_FACTOR: f64 = 0.5;
/// A shape whose centre lines are shorter than this many times its width is a compact blob
/// (estimated).
pub const COMPACT: f64 = 2.0;
/// How far a column reaches into the fill it meets, mm, so no gap shows (estimated).
pub const FILL_OVERLAP_MM: f64 = 0.3;
/// The most fill outline points one shape gives (a hostile outline is filled as drawn past it).
const MAX_RING_POINTS: usize = 200_000;

/// Auto-digitize thresholds, mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoParams {
    /// Stretches no wider than this are running stitch.
    pub min_column: f64,
    /// Stretches wider than this are tatami fill.
    pub max_satin: f64,
}

impl Default for AutoParams {
    fn default() -> Self {
        Self { min_column: MIN_COLUMN_MM, max_satin: MAX_SATIN_MM }
    }
}

/// What a shape becomes, as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoKind {
    /// Nothing to sew (no area).
    None,
    /// Running stitch only.
    Run,
    /// Satin columns (thin stretches as running stitch).
    Satin,
    /// One tatami fill.
    Fill,
    /// A fill with columns or runs off it.
    Mixed,
}

impl AutoKind {
    pub fn id(self) -> &'static str {
        match self {
            AutoKind::None => "none",
            AutoKind::Run => "run",
            AutoKind::Satin => "satin",
            AutoKind::Fill => "fill",
            AutoKind::Mixed => "mixed",
        }
    }
}

/// The stitch objects a shape becomes.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoShape {
    pub kind: AutoKind,
    /// Two-rail satin columns.
    pub columns: Vec<Column>,
    /// Running stitch lines.
    pub runs: Vec<Vec<P>>,
    /// The fill's outline (rings, nonzero rule); empty without a fill.
    pub fill: Vec<Vec<P>>,
}

impl AutoShape {
    fn of(kind: AutoKind) -> AutoShape {
        AutoShape { kind, columns: vec![], runs: vec![], fill: vec![] }
    }
}

/// How an open stroked path is sewn, by its stroke weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Run,
    /// A satin column the stroke weight wide.
    Satin,
    /// Too wide for satin: the stroke's outline filled.
    Fill,
}

/// How a stroke `weight` mm wide is sewn: running stitch up to the minimum column, satin up to
/// the widest satin, else fill.
pub fn stroke_kind(weight: f64, p: &AutoParams) -> StrokeKind {
    if weight.is_nan() || weight <= p.min_column {
        StrokeKind::Run
    } else if weight <= p.max_satin {
        StrokeKind::Satin
    } else {
        StrokeKind::Fill
    }
}

/// The length of a polyline.
fn length(pts: &[P]) -> f64 {
    pts.windows(2).map(|w| if let [a, b] = w { dist(*a, *b) } else { 0.0 }).sum()
}

/// The pixel of grid `g` holding `q`.
fn pixel(g: &Grid, q: P) -> Option<usize> {
    let fx = ((q.0 - g.x0) / g.px).floor();
    let fy = ((q.1 - g.y0) / g.px).floor();
    if !(fx.is_finite() && fy.is_finite()) || fx < 0.0 || fy < 0.0 || fx >= g.w as f64 || fy >= g.h as f64 {
        return None;
    }
    // In range, so the casts are exact.
    Some(fy as usize * g.w + fx as usize)
}

/// Columns and runs of stroke `s` over its points `from..=to`.
fn stretch(s: &Stroke, from: usize, to: usize) -> Option<Stroke> {
    Some(Stroke {
        a: s.a,
        b: s.b,
        centre: s.centre.get(from..=to)?.to_vec(),
        left: s.left.get(from..=to)?.to_vec(),
        right: s.right.get(from..=to)?.to_vec(),
        column: s.column.get(from..=to)?.to_vec(),
    })
}

/// The stretches of `s` whose points `covered` doesn't hold, as (from, to) point indices.
fn uncovered(s: &Stroke, covered: &dyn Fn(P) -> bool) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut start: Option<usize> = None;
    for (i, q) in s.centre.iter().enumerate() {
        match (covered(*q), start) {
            (false, None) => start = Some(i),
            (true, Some(a)) => {
                out.push((a, i.saturating_sub(1)));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start {
        out.push((a, s.centre.len().saturating_sub(1)));
    }
    out.into_iter().filter(|(a, b)| b > a).collect()
}

/// The stitch objects closed outline `rings` (mm, nonzero rule) becomes. Hostile outlines are
/// bounded as lettering's are: non-finite points or too many segments are an error.
pub fn auto_shape(rings: &[Vec<P>], p: &AutoParams) -> Result<AutoShape, String> {
    let Some(sk) = columns::skeleton_of(rings, "shape")? else { return Ok(AutoShape::of(AutoKind::None)) };
    let whole_fill = || AutoShape { fill: rings.to_vec(), ..AutoShape::of(AutoKind::Fill) };
    let Some(gr) = sk.graph else { return Ok(whole_fill()) };
    let g = &sk.grid;
    // A column only where it is measurably (a pixel) wider than the minimum.
    let strokes = columns::strokes(&gr, g, &sk.segs, |w| w > p.min_column + g.px);
    // The wide part: within half the widest satin of a pixel whose half width is at least that.
    let r_px = p.max_satin / 2.0 / g.px;
    let centres: Vec<bool> = g.dist.iter().map(|d| d - 0.5 >= r_px).collect();
    if !centres.contains(&true) {
        // No fill: a compact blob is filled whole, else columns and runs.
        let widest = strokes.iter().flat_map(|s| s.left.iter().zip(&s.right).map(|(a, b)| dist(*a, *b))).fold(0.0, f64::max);
        let total: f64 = strokes.iter().map(|s| length(&s.centre)).sum();
        if widest >= p.max_satin / 2.0 && total < COMPACT * widest {
            return Ok(whole_fill());
        }
        let mut out = AutoShape::of(AutoKind::Run);
        for s in &strokes {
            add_pieces(&mut out, s);
        }
        out.kind = if out.columns.is_empty() { AutoKind::Run } else { AutoKind::Satin };
        if out.columns.is_empty() && out.runs.is_empty() {
            return Ok(whole_fill());
        }
        return Ok(out);
    }
    let not_c: Vec<bool> = centres.iter().map(|c| !c).collect();
    // Pixels from the nearest wide centre.
    let dc = skeleton::distance(&not_c, g.w, g.h);
    let inside = |i: usize| g.inside.get(i).copied().unwrap_or(false);
    let in_open = |i: usize| inside(i) && dc.get(i).is_some_and(|d| *d <= r_px);
    let deep = (r_px - FILL_OVERLAP_MM / g.px).max(0.0);
    let covered = |q: P| pixel(g, q).is_some_and(|i| inside(i) && dc.get(i).is_some_and(|d| *d <= deep));
    // Leftover stretches, the short ones dropped.
    let stub = STUB_FACTOR * p.max_satin;
    let mut out = AutoShape::of(AutoKind::Mixed);
    let mut kept_px: Vec<usize> = vec![];
    for s in &strokes {
        for (a, b) in uncovered(s, &covered) {
            let Some(part) = stretch(s, a, b) else { continue };
            if length(&part.centre) < stub {
                continue;
            }
            kept_px.extend(part.centre.iter().filter_map(|q| pixel(g, *q)));
            add_pieces(&mut out, &part);
        }
    }
    if out.columns.is_empty() && out.runs.is_empty() {
        return Ok(whole_fill());
    }
    // The fill: the shape less the leftover areas a kept stretch runs through.
    let mut fill: Vec<bool> = (0..g.w * g.h).map(in_open).collect();
    let mut label = vec![0u32; g.w * g.h];
    let mut keep = vec![false];
    let mut stack = vec![];
    for i in 0..g.w * g.h {
        if !inside(i) || in_open(i) || label.get(i).copied().unwrap_or(1) != 0 {
            continue;
        }
        let id = u32::try_from(keep.len()).unwrap_or(u32::MAX);
        keep.push(false);
        stack.push(i);
        if let Some(l) = label.get_mut(i) {
            *l = id;
        }
        while let Some(c) = stack.pop() {
            let (x, y) = (c % g.w, c / g.w);
            let nbrs = [(x > 0).then(|| c - 1), (x + 1 < g.w).then_some(c + 1), (y > 0).then(|| c - g.w), (y + 1 < g.h).then_some(c + g.w)];
            for n in nbrs.into_iter().flatten() {
                if inside(n) && !in_open(n) && label.get(n) == Some(&0) {
                    if let Some(l) = label.get_mut(n) {
                        *l = id;
                    }
                    stack.push(n);
                }
            }
        }
    }
    for i in kept_px {
        if let Some(k) = label.get(i).and_then(|l| keep.get_mut(*l as usize)) {
            *k = true;
        }
    }
    for (i, f) in fill.iter_mut().enumerate() {
        if !*f && inside(i) {
            *f = label.get(i).and_then(|l| keep.get(*l as usize)).is_some_and(|k| !*k);
        }
    }
    let rings_px = trace_mask(&fill, g.w, g.h);
    let mut points = 0usize;
    for r in rings_px {
        let r = simplify(&r, 0.75);
        points += r.len();
        if points > MAX_RING_POINTS {
            return Ok(whole_fill());
        }
        if r.len() >= 3 {
            out.fill.push(r.iter().map(|&(x, y)| (g.x0 + x * g.px, g.y0 + y * g.px)).collect());
        }
    }
    if out.fill.is_empty() {
        out.kind = if out.columns.is_empty() { AutoKind::Run } else { AutoKind::Satin };
    }
    Ok(out)
}

/// The columns and runs of stroke `s` added to `out`.
fn add_pieces(out: &mut AutoShape, s: &Stroke) {
    for (col, from, to) in pieces(s) {
        if col {
            if let (Some(l), Some(r)) = (s.left.get(from..=to), s.right.get(from..=to)) {
                out.columns.push(Column { left: l.to_vec(), right: r.to_vec() });
            }
        } else if let Some(c) = s.centre.get(from..=to) {
            out.runs.push(c.to_vec());
        }
    }
}

/// Directions on the pixel-corner grid, y down.
const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// The boundary loops of `mask` (w × h pixels) as pixel-corner points, each edge's midpoint (so
/// steps become 45° chamfers): outer boundaries clockwise on screen, holes the other way, so the
/// nonzero rule fills what the mask holds. At a saddle the walk turns right, keeping pixels that
/// touch only at a corner apart.
fn trace_mask(mask: &[bool], w: usize, h: usize) -> Vec<Vec<(f64, f64)>> {
    let on =
        |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask.get(y as usize * w + x as usize).copied().unwrap_or(false);
    // Each edge: start corner, direction index; inside on its right.
    let mut edges: Vec<((i32, i32), usize)> = vec![];
    for y in 0..h {
        for x in 0..w {
            if !mask.get(y * w + x).copied().unwrap_or(false) {
                continue;
            }
            // The grid is at most about 800 a side, so these fit.
            let (xi, yi) = (x as i32, y as i32);
            let (xl, yl) = (x as i64, y as i64);
            if !on(xl, yl - 1) {
                edges.push(((xi, yi), 0));
            }
            if !on(xl + 1, yl) {
                edges.push(((xi + 1, yi), 1));
            }
            if !on(xl, yl + 1) {
                edges.push(((xi + 1, yi + 1), 2));
            }
            if !on(xl - 1, yl) {
                edges.push(((xi, yi + 1), 3));
            }
        }
    }
    let mut from: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        from.entry(e.0).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut loops = vec![];
    for first in 0..edges.len() {
        if used.get(first).copied().unwrap_or(true) {
            continue;
        }
        let mut pts = vec![];
        let mut cur = first;
        for _ in 0..=edges.len() {
            if let Some(u) = used.get_mut(cur) {
                *u = true;
            }
            let Some(&(s, d)) = edges.get(cur) else { break };
            let Some(&(dx, dy)) = DIRS.get(d) else { break };
            pts.push((f64::from(s.0) + f64::from(dx) / 2.0, f64::from(s.1) + f64::from(dy) / 2.0));
            let end = (s.0 + dx, s.1 + dy);
            let Some(cands) = from.get(&end) else { break };
            // Right turn, then straight on, then left.
            let pick = [(d + 1) % 4, d, (d + 3) % 4]
                .into_iter()
                .find_map(|nd| cands.iter().copied().find(|&c| edges.get(c).is_some_and(|e| e.1 == nd) && !used.get(c).copied().unwrap_or(true)));
            match pick {
                Some(n) => cur = n,
                None => break,
            }
        }
        if pts.len() >= 3 {
            loops.push(pts);
        }
    }
    loops
}

/// Ring `r` simplified (Douglas–Peucker, `tol` pixels), kept closed.
fn simplify(r: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    let n = r.len();
    if n < 4 {
        return r.to_vec();
    }
    let Some(&p0) = r.first() else { return vec![] };
    // Split at the point farthest from the first.
    let far = (1..n).max_by(|&a, &b| r.get(a).map_or(0.0, |q| dist(p0, *q)).total_cmp(&r.get(b).map_or(0.0, |q| dist(p0, *q)))).unwrap_or(n / 2);
    let mut keep = vec![false; n + 1];
    if let Some(k) = keep.get_mut(0) {
        *k = true;
    }
    if let Some(k) = keep.get_mut(far) {
        *k = true;
    }
    let at = |i: usize| r.get(i % n).copied().unwrap_or(p0);
    let mut stack = vec![(0usize, far), (far, n)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (at(a), at(b));
        let (mut best, mut bi) = (0.0, a);
        for i in a + 1..b {
            let d = crate::fill::seg_dist(at(i), pa, pb);
            if d > best {
                best = d;
                bi = i;
            }
        }
        if best > tol {
            if let Some(k) = keep.get_mut(bi) {
                *k = true;
            }
            stack.push((a, bi));
            stack.push((bi, b));
        }
    }
    (0..n).filter(|&i| keep.get(i).copied().unwrap_or(false)).map(at).collect()
}

#[cfg(test)]
mod tests;
