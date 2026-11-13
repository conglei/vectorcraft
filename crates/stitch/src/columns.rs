//! Centre lines and rails of a closed outline: the skeleton graph of [`crate::skeleton`] turned
//! into strokes, each with a smoothed centre line, two rails cast to the outline and a flag per
//! point saying whether it is part of a satin column. Shared by lettering ([`crate::lettering`])
//! and auto-digitizing ([`crate::autodigitize`]).

use crate::lettering::P;
use crate::satin::resample;
use crate::skeleton::{self, Graph, Grid};

/// Centre line sampling, mm.
pub(crate) const STEP_MM: f64 = 0.25;
/// The most strokes one outline may have; more (a hostile or very ornate outline) is sewn as fill.
pub(crate) const MAX_EDGES: usize = 400;
/// Spurs shorter than this many junction half widths are thinning leftovers.
const SPUR: f64 = 1.2;
/// Rails are held to this many local half widths from the centre line.
const RAIL_HOLD: f64 = 1.6;
/// Free ends stop this far inside the outline, mm.
const END_INSET_MM: f64 = 0.1;

pub(crate) fn dist(a: P, b: P) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

pub(crate) fn finite(p: P) -> bool {
    p.0.is_finite() && p.1.is_finite()
}

/// An outline analysed: its segments, its grid and its pruned skeleton graph (`None` when it has
/// more than [`MAX_EDGES`] strokes).
pub(crate) struct Skeleton {
    pub(crate) segs: Vec<(P, P)>,
    pub(crate) grid: Grid,
    pub(crate) graph: Option<Graph>,
}

/// The outline's segments, once its points are checked to be numbers and its segments to be at
/// most [`skeleton::MAX_SEGMENTS`] (the error names `what` the outline is).
pub(crate) fn check_outline(rings: &[Vec<P>], what: &str) -> Result<Vec<(P, P)>, String> {
    if rings.iter().flatten().any(|q| !finite(*q)) {
        return Err(format!("the {what}'s outline has points that aren't numbers"));
    }
    let segs = skeleton::segments(rings);
    if segs.len() > skeleton::MAX_SEGMENTS {
        return Err(format!("the {what}'s outline has too many points"));
    }
    Ok(segs)
}

/// The skeleton of the outline `rings` (mm, nonzero rule); `Ok(None)` for an outline of no area.
/// Hostile outlines are bounded: non-finite points or more than [`skeleton::MAX_SEGMENTS`]
/// segments are an error naming `what` the outline is.
pub(crate) fn skeleton_of(rings: &[Vec<P>], what: &str) -> Result<Option<Skeleton>, String> {
    let segs = check_outline(rings, what)?;
    let Some(grid) = skeleton::rasterize(&segs) else { return Ok(None) };
    let skel = skeleton::thin(&grid);
    let graph = skeleton::graph(&grid, &skel, MAX_EDGES).map(|mut gr| {
        skeleton::prune(&mut gr, SPUR, grid.px);
        gr
    });
    Ok(Some(Skeleton { segs, grid, graph }))
}

/// A stroke ready to sew: its centre line from node `a` to node `b`, its rails, and whether each
/// point is part of a satin column.
#[derive(Clone, Debug)]
pub(crate) struct Stroke {
    pub(crate) a: usize,
    pub(crate) b: usize,
    pub(crate) centre: Vec<P>,
    pub(crate) left: Vec<P>,
    pub(crate) right: Vec<P>,
    pub(crate) column: Vec<bool>,
}

/// Stretches of one kind: (column?, from, to) point indices. Every rung of a column piece is a
/// column point; run pieces reach to the end points of their neighbours, so the line stays
/// continuous.
pub(crate) fn pieces(s: &Stroke) -> Vec<(bool, usize, usize)> {
    let n = s.centre.len();
    let flag = |i: usize| s.column.get(i).copied().unwrap_or(false);
    let mut blocks: Vec<(bool, usize, usize)> = vec![];
    let mut start = 0usize;
    for i in 1..=n {
        if i == n || flag(i) != flag(start) {
            // A column too short to sew as satin (under three rungs) is sewn as a run.
            let col = flag(start) && i >= start + 3;
            match blocks.last_mut() {
                Some(b) if b.0 == col => b.2 = i - 1,
                _ => blocks.push((col, start, i - 1)),
            }
            start = i;
        }
    }
    let last = n.saturating_sub(1);
    blocks
        .into_iter()
        .map(|(col, a, b)| if col { (col, a, b) } else { (col, a.saturating_sub(1), (b + 1).min(last)) })
        .filter(|p| p.2 > p.1)
        .collect()
}

/// The nearest hit of the ray from `o` along `d` (a unit vector) with `segs`.
pub(crate) fn ray(segs: &[(P, P)], o: P, d: P) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &(a, b) in segs {
        let e = (b.0 - a.0, b.1 - a.1);
        let den = d.0 * e.1 - d.1 * e.0;
        if den.abs() < 1e-12 {
            continue;
        }
        let w = (a.0 - o.0, a.1 - o.1);
        let t = (w.0 * e.1 - w.1 * e.0) / den;
        let u = (w.0 * d.1 - w.1 * d.0) / den;
        if t > 1e-9 && (-1e-9..=1.0 + 1e-9).contains(&u) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best
}

/// `pts` smoothed (ends kept) and resampled every [`STEP_MM`].
fn smooth(pts: &[P]) -> Vec<P> {
    let mut v = pts.to_vec();
    for _ in 0..4 {
        let prev = v.clone();
        for i in 1..prev.len().saturating_sub(1) {
            if let (Some(a), Some(b), Some(c), Some(o)) = (prev.get(i - 1), prev.get(i), prev.get(i + 1), v.get_mut(i)) {
                *o = ((a.0 + 2.0 * b.0 + c.0) / 4.0, (a.1 + 2.0 * b.1 + c.1) / 4.0);
            }
        }
    }
    let len: f64 = v.windows(2).map(|w| if let [a, b] = w { dist(*a, *b) } else { 0.0 }).sum();
    // At most a few thousand: the grid is at most 800 pixels a side.
    let n = ((len / STEP_MM).ceil() as usize).clamp(1, 20_000) + 1;
    resample(&v, n)
}

/// The unit tangent at point `i` of `pts`.
fn tangent(pts: &[P], i: usize) -> P {
    let a = pts.get(i.saturating_sub(1)).or(pts.get(i));
    let b = pts.get(i + 1).or(pts.get(i));
    match (a, b) {
        (Some(a), Some(b)) => {
            let l = dist(*a, *b);
            if l > 1e-12 { ((b.0 - a.0) / l, (b.1 - a.1) / l) } else { (1.0, 0.0) }
        }
        _ => (1.0, 0.0),
    }
}

/// Extend `pts` past its last point along its tangent to just inside the outline, at most
/// `reach`.
fn extend_end(pts: &mut Vec<P>, segs: &[(P, P)], reach: f64) {
    let n = pts.len();
    let (Some(&end), Some(&back)) = (pts.last(), pts.get(n.saturating_sub(1 + (reach / STEP_MM).ceil().clamp(1.0, 8.0) as usize))) else { return };
    let l = dist(back, end);
    if l < 1e-9 {
        return;
    }
    let d = ((end.0 - back.0) / l, (end.1 - back.1) / l);
    let Some(t) = ray(segs, end, d) else { return };
    let go = (t - END_INSET_MM).min(reach);
    if go <= STEP_MM / 2.0 {
        return;
    }
    let k = (go / STEP_MM).ceil().clamp(1.0, 100.0) as usize;
    for i in 1..=k {
        let f = go * i as f64 / k as f64;
        pts.push((end.0 + d.0 * f, end.1 + d.1 * f));
    }
}

/// The node positions, with room for the nodes [`strokes`] added for dots.
pub(crate) fn extend_nodes(mut nodes: Vec<P>, strokes: &[Stroke]) -> Vec<P> {
    for s in strokes {
        for (n, q) in [(s.a, s.centre.first()), (s.b, s.centre.last())] {
            while nodes.len() <= n {
                nodes.push(q.copied().unwrap_or((0.0, 0.0)));
            }
        }
    }
    nodes
}

/// The strokes of graph `gr`: centre lines smoothed, free ends extended to the outline, rails
/// cast to the outline; a dot (a node with no strokes) becomes a short stroke across it. A point
/// is part of a satin column when `column_at` says so of its width as drawn, rail to rail (mm).
pub(crate) fn strokes(gr: &Graph, grid: &Grid, segs: &[(P, P)], column_at: impl Fn(f64) -> bool) -> Vec<Stroke> {
    let mut out = vec![];
    let mut next_node = gr.nodes.len();
    let mut lines: Vec<(usize, usize, Vec<P>)> = vec![];
    for (_, e) in gr.live() {
        let mut c = smooth(&e.pts);
        if c.len() < 2 {
            continue;
        }
        let reach = |n: usize| gr.nodes.get(n).map_or(0.0, |x| x.r) * 3.0 + 0.5;
        if e.a != e.b && gr.degree(e.b) == 1 {
            extend_end(&mut c, segs, reach(e.b));
        }
        if e.a != e.b && gr.degree(e.a) == 1 {
            c.reverse();
            extend_end(&mut c, segs, reach(e.a));
            c.reverse();
        }
        lines.push((e.a, e.b, c));
    }
    for (i, n) in gr.nodes.iter().enumerate() {
        if !n.joined && gr.degree(i) == 0 {
            let r = n.r.max(grid.px);
            let (a, b) = ((n.pos.0 - r, n.pos.1), (n.pos.0 + r, n.pos.1));
            let k = ((2.0 * r / STEP_MM).ceil() as usize).clamp(1, 1000);
            let c: Vec<P> = (0..=k).map(|j| (a.0 + (b.0 - a.0) * j as f64 / k as f64, a.1)).collect();
            lines.push((next_node, next_node + 1, c));
            next_node += 2;
        }
    }
    for (a, b, c) in lines {
        let hw: Vec<f64> = c.iter().map(|&q| grid.half_width(q)).collect();
        let typical = {
            let mut v: Vec<f64> = hw.iter().copied().filter(|&h| h > 0.0).collect();
            v.sort_by(f64::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(grid.px)
        };
        let (mut left, mut right) = (Vec::with_capacity(c.len()), Vec::with_capacity(c.len()));
        let mut column = Vec::with_capacity(c.len());
        for (i, &q) in c.iter().enumerate() {
            let window = hw.get(i.saturating_sub(4)..(i + 5).min(hw.len())).unwrap_or(&[]);
            let local = window.iter().copied().fold(0.0, f64::max);
            let local = if local > 0.0 { local } else { typical };
            let hold = RAIL_HOLD * local + grid.px;
            let t = tangent(&c, i);
            let n = (-t.1, t.0);
            let dl = ray(segs, q, n).map_or(local, |d| d.min(hold));
            let dr = ray(segs, q, (-n.0, -n.1)).map_or(local, |d| d.min(hold));
            left.push((q.0 + n.0 * dl, q.1 + n.1 * dl));
            right.push((q.0 - n.0 * dr, q.1 - n.1 * dr));
            column.push(column_at(dl + dr));
        }
        out.push(Stroke { a, b, centre: c, left, right, column });
    }
    trim_branches(&mut out, gr);
    out
}

/// How far a branch's column reaches into the stroke it meets, mm: enough overlap that no gap
/// shows, without its stitches lying across the other column.
const BRANCH_OVERLAP_MM: f64 = 0.2;

/// At a junction where two strokes run on through (the stem of an H, the bar of a T), the other
/// strokes meeting there are branches: their columns start at the through stroke's edge, a
/// little inside it, rather than at its centre, where their stitches would lie across it.
fn trim_branches(strokes: &mut [Stroke], gr: &Graph) {
    for (n, node) in gr.nodes.iter().enumerate() {
        // The stroke ends at this node: (stroke, at its start?), with the direction into it.
        let mut ends: Vec<(usize, bool, P)> = vec![];
        for (i, s) in strokes.iter().enumerate() {
            for (start, at) in [(true, s.a), (false, s.b)] {
                if at != n || (s.a == s.b && !start) {
                    continue;
                }
                let k = ((1.0 / STEP_MM) as usize).min(s.centre.len().saturating_sub(1));
                let (p0, p1) =
                    if start { (s.centre.first(), s.centre.get(k)) } else { (s.centre.last(), s.centre.get(s.centre.len().saturating_sub(1 + k))) };
                if let (Some(p0), Some(p1)) = (p0, p1) {
                    let l = dist(*p0, *p1);
                    if l > 1e-9 {
                        ends.push((i, start, ((p1.0 - p0.0) / l, (p1.1 - p0.1) / l)));
                    }
                }
            }
        }
        if ends.len() < 3 {
            continue;
        }
        // The most nearly opposite pair runs through, when it bends by less than 60 degrees.
        let mut best: Option<(usize, usize, f64)> = None;
        for x in 0..ends.len() {
            for y in x + 1..ends.len() {
                let (Some(e1), Some(e2)) = (ends.get(x), ends.get(y)) else { continue };
                let dot = e1.2.0 * e2.2.0 + e1.2.1 * e2.2.1;
                if dot < -0.5 && best.is_none_or(|b| dot < b.2) {
                    best = Some((x, y, dot));
                }
            }
        }
        let Some((x, y, _)) = best else { continue };
        let reach = node.r - BRANCH_OVERLAP_MM;
        if reach <= 0.0 {
            continue;
        }
        for (k, &(i, start, _)) in ends.iter().enumerate() {
            if k == x || k == y {
                continue;
            }
            let Some(s) = strokes.get_mut(i) else { continue };
            let n = s.centre.len();
            for j in 0..n {
                let idx = if start { j } else { n - 1 - j };
                let (Some(q), Some(flag)) = (s.centre.get(idx).copied(), s.column.get_mut(idx)) else { break };
                if dist(q, node.pos) >= reach {
                    break;
                }
                *flag = false;
            }
        }
    }
}
