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
        join_corners(&mut gr, &grid);
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
    /// Whether each centre point is a corner the line turns (running stitch keeps it).
    pub(crate) bend: Vec<bool>,
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
/// `reach`. Within about the half width `h` of a free end the skeleton bends off towards a corner
/// of the end (thinning's leftovers); that stretch is dropped first, so the line runs on straight
/// and the end's rungs lie square across it.
fn extend_end(pts: &mut Vec<P>, segs: &[(P, P)], reach: f64, h: f64) {
    let drop = h.min(len_of(pts) * 0.3).max(0.0);
    let mut gone = 0.0;
    while pts.len() > 2 && gone < drop {
        let (Some(&a), Some(&b)) = (pts.get(pts.len() - 2), pts.last()) else { break };
        gone += dist(a, b);
        pts.pop();
    }
    let reach = reach + gone;
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

/// Two strokes meeting at a corner are joined when their widths are within this factor.
const ALIKE: f64 = 2.0;
/// A mitre reaching further than this many half widths from the centre line is capped.
const MITRE_LIMIT: f64 = 2.0;
/// A stroke from a corner to a free end shorter than this many half widths of the corner is the
/// stub of a flat point, part of the corner (estimated).
const STUB: f64 = 2.5;

/// The median of `v` (0 when empty).
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// Join the strokes meeting at each corner left in graph `gr` (a node where just two meet, at a
/// bend, [`skeleton::prune`] having joined the straight ones) when they are alike in width, so
/// one column turns the corner; and turn a ring whose own node is a corner to start in the middle
/// of its longest side.
fn join_corners(gr: &mut Graph, grid: &Grid) {
    let width = |e: &skeleton::Edge| median(e.pts.iter().map(|q| grid.half_width(*q)).collect());
    // A sharp corner cut flat (the point of a V) leaves a short stub from the corner to the flat:
    // it is part of the corner, which the cap covers, not a stroke of its own.
    for n in 0..gr.nodes.len() {
        let Some(node) = gr.nodes.get(n) else { continue };
        let r = node.r;
        let live: Vec<usize> = node.edges.iter().copied().filter(|&e| gr.edges.get(e).is_some_and(|x| x.alive && x.a != x.b)).collect();
        if live.len() != 3 || gr.degree(n) != 3 {
            continue;
        }
        let stub = |e: usize| {
            gr.edges.get(e).is_some_and(|x| {
                let other = if x.a == n { x.b } else { x.a };
                gr.degree(other) == 1 && x.length() < STUB * r + grid.px
            })
        };
        let stubs: Vec<usize> = live.iter().copied().filter(|&e| stub(e)).collect();
        let [s] = stubs.as_slice() else { continue };
        let rest: Vec<usize> = live.iter().copied().filter(|e| e != s).collect();
        let [e1, e2] = rest.as_slice() else { continue };
        let (Some(w1), Some(w2)) = (gr.edges.get(*e1).map(width), gr.edges.get(*e2).map(width)) else { continue };
        if w1 <= 0.0 || w2 <= 0.0 || w1.max(w2) > ALIKE * w1.min(w2) {
            continue;
        }
        let Some(spur) = gr.edges.get_mut(*s) else { continue };
        spur.alive = false;
        let (a, b, s) = (spur.a, spur.b, *s);
        for end in [a, b] {
            if let Some(x) = gr.nodes.get_mut(end) {
                x.edges.retain(|&e| e != s);
            }
        }
        // Straight on through it, the stub was a serif's or a branch's: keep it.
        if skeleton::runs_on(gr, n, grid.px) {
            if let Some(spur) = gr.edges.get_mut(s) {
                spur.alive = true;
            }
            for end in [a, b] {
                if let Some(x) = gr.nodes.get_mut(end) {
                    x.edges.push(s);
                }
            }
        }
    }
    for n in 0..gr.nodes.len() {
        let live: Vec<usize> = gr
            .nodes
            .get(n)
            .map_or(vec![], |node| node.edges.iter().copied().filter(|&e| gr.edges.get(e).is_some_and(|x| x.alive && x.a != x.b)).collect());
        let [e1, e2] = live.as_slice() else { continue };
        if gr.degree(n) != 2 {
            continue;
        }
        let (Some(w1), Some(w2)) = (gr.edges.get(*e1).map(width), gr.edges.get(*e2).map(width)) else { continue };
        if w1 <= 0.0 || w2 <= 0.0 || w1.max(w2) > ALIKE * w1.min(w2) {
            continue;
        }
        skeleton::merge(gr, n, grid.px);
    }
    for e in 0..gr.edges.len() {
        let Some(edge) = gr.edges.get(e) else { continue };
        if !edge.alive || edge.a != edge.b || gr.degree(edge.a) != 2 || edge.pts.len() < 8 {
            continue;
        }
        let n = edge.a;
        let pts = &edge.pts;
        let Some(&o) = pts.first() else { continue };
        let reach = (gr.nodes.get(n).map_or(0.0, |x| x.r).max(grid.px) * 2.0).max(4.0 * grid.px);
        let away = |q: Option<&P>| -> Option<P> { unit(o, *q?) };
        let d1 = away(pts.iter().find(|q| dist(o, **q) >= reach));
        let d2 = away(pts.iter().rev().find(|q| dist(o, **q) >= reach));
        let (Some(d1), Some(d2)) = (d1, d2) else { continue };
        // Straight on through the closing point: no corner there.
        if d1.0 * d2.0 + d1.1 * d2.1 < -std::f64::consts::FRAC_1_SQRT_2 {
            continue;
        }
        // Start in the middle of the longest side between corners instead.
        let mut cuts: Vec<usize> = vec![0];
        cuts.extend(edge.corners.iter().filter_map(|c| index_of(pts, *c)));
        cuts.push(pts.len() - 1);
        cuts.sort_unstable();
        let arc = |a: usize, b: usize| pts.get(a..=b).map_or(0.0, len_of);
        let sides = cuts.windows(2).filter_map(|w| if let [a, b] = w { Some((*a, *b)) } else { None });
        let Some((a, b)) = sides.max_by(|x, y| arc(x.0, x.1).total_cmp(&arc(y.0, y.1))) else { continue };
        let k = (a + b) / 2;
        if k == 0 || k + 1 >= pts.len() {
            continue;
        }
        let mut rotated: Vec<P> = pts.get(k..pts.len() - 1).unwrap_or(&[]).to_vec();
        rotated.extend(pts.get(..=k).unwrap_or(&[]));
        let mut corners = edge.corners.clone();
        corners.push(o);
        let start = rotated.first().copied().unwrap_or(o);
        if let Some(ed) = gr.edges.get_mut(e) {
            ed.pts = rotated;
            ed.corners = corners;
        }
        if let Some(node) = gr.nodes.get_mut(n) {
            node.pos = start;
        }
    }
}

/// The index of the point of `pts` at `c` (within a hair).
fn index_of(pts: &[P], c: P) -> Option<usize> {
    let (i, d) = pts.iter().enumerate().map(|(i, q)| (i, dist(*q, c))).min_by(|x, y| x.1.total_cmp(&y.1))?;
    (d < 1e-6).then_some(i)
}

/// The length of a polyline.
fn len_of(pts: &[P]) -> f64 {
    pts.windows(2).map(|w| if let [a, b] = w { dist(*a, *b) } else { 0.0 }).sum()
}

/// The unit vector from `a` to `b`.
fn unit(a: P, b: P) -> Option<P> {
    let l = dist(a, b);
    (l > 1e-12).then(|| ((b.0 - a.0) / l, (b.1 - a.1) / l))
}

/// One side of a stroke between corners, its rungs cast: centre, left and right rails, and
/// whether each rung is wide enough for a column.
struct Arm {
    centre: Vec<P>,
    left: Vec<P>,
    right: Vec<P>,
    column: Vec<bool>,
}

impl Arm {
    /// Keep the rungs `0..=hi` (`at_end`) or `lo..`, at least one.
    fn keep(&mut self, i: usize, at_end: bool) {
        let n = self.centre.len();
        let (lo, hi) = if at_end { (0, i.min(n.saturating_sub(1))) } else { (i.min(n.saturating_sub(1)), n.saturating_sub(1)) };
        for v in [&mut self.centre, &mut self.left, &mut self.right] {
            *v = v.get(lo..=hi).map(<[P]>::to_vec).unwrap_or_default();
        }
        self.column = self.column.get(lo..=hi).map(<[bool]>::to_vec).unwrap_or_default();
    }

    /// Cast again the rungs within `reach` of the end (`at_end`) or the start, square to `dir`,
    /// each rail held as far out as it was or at the median half width's [`RAIL_HOLD`].
    fn square(&mut self, dir: P, reach: f64, at_end: bool, segs: &[(P, P)]) {
        let hold = RAIL_HOLD * self.half_width() + STEP_MM;
        let n = (-dir.1, dir.0);
        let count = self.centre.len();
        let mut gone = 0.0;
        for k in 0..count {
            let i = if at_end { count - 1 - k } else { k };
            if k > 0 {
                let j = if at_end { i + 1 } else { i - 1 };
                if let (Some(a), Some(b)) = (self.centre.get(i), self.centre.get(j)) {
                    gone += dist(*a, *b);
                }
            }
            if gone > reach {
                break;
            }
            let Some(&q) = self.centre.get(i) else { break };
            let dl = ray(segs, q, n).map_or(hold, |d| d.min(hold));
            let dr = ray(segs, q, (-n.0, -n.1)).map_or(hold, |d| d.min(hold));
            if let (Some(l), Some(r)) = (self.left.get_mut(i), self.right.get_mut(i)) {
                *l = (q.0 + n.0 * dl, q.1 + n.1 * dl);
                *r = (q.0 - n.0 * dr, q.1 - n.1 * dr);
            }
        }
    }

    /// The median half width of its rungs.
    fn half_width(&self) -> f64 {
        median(self.left.iter().zip(&self.right).map(|(l, r)| dist(*l, *r) / 2.0).collect())
    }
}

/// The point `x` along `pts` from its end (`from_end`) or its start, the far end when shorter.
fn along(pts: &[P], x: f64, from_end: bool) -> Option<P> {
    let mut it: Box<dyn Iterator<Item = &P>> = if from_end { Box::new(pts.iter().rev()) } else { Box::new(pts.iter()) };
    let mut prev = *it.next()?;
    let mut gone = 0.0;
    for &q in it {
        let l = dist(prev, q);
        if gone + l >= x && l > 0.0 {
            let t = (x - gone) / l;
            return Some((prev.0 + (q.0 - prev.0) * t, prev.1 + (q.1 - prev.1) * t));
        }
        gone += l;
        prev = q;
    }
    Some(prev)
}

/// Where the lines through `p` along `d` and through `q` along `e` meet.
fn meet(p: P, d: P, q: P, e: P) -> Option<P> {
    let den = d.0 * e.1 - d.1 * e.0;
    if den.abs() < 1e-9 {
        return None;
    }
    let s = ((q.0 - p.0) * e.1 - (q.1 - p.1) * e.0) / den;
    let m = (p.0 + d.0 * s, p.1 + d.1 * s);
    finite(m).then_some(m)
}

/// A corner's rungs: (centre, left, right).
type Turn = (Vec<P>, Vec<P>, Vec<P>);

/// The rungs that turn the corner near `c` (the skeleton's corner) between `first` (arriving)
/// and `second` (leaving). The arms' directions are taken a little back from the corner, where
/// the skeleton has not yet bent into it, and the corner put where their centre lines meet. Both
/// arms are cut back where their normals would reach across into each other; then the inner rail
/// folds at the inner corner and the outer rail goes round a mitre (or, past [`MITRE_LIMIT`], a
/// square cap), each held inside the outline `segs`. The two halves are paired apart, so the rung
/// from the inner corner to the mitre is the corner's diagonal. Returns the rungs between the
/// arms' own.
fn corner(first: &mut Arm, second: &mut Arm, c: P, segs: &[(P, P)]) -> Option<Turn> {
    let h = ((first.half_width() + second.half_width()) / 2.0).max(STEP_MM / 2.0);
    // Directions over the stretch from one to three half widths out from the corner.
    let (p_in, p_out) = (along(&first.centre, h, true)?, along(&second.centre, h, false)?);
    let t_in = unit(along(&first.centre, 3.0 * h, true)?, p_in).or_else(|| heading(&first.centre, false))?;
    let t_out = unit(p_out, along(&second.centre, 3.0 * h, false)?).or_else(|| heading(&second.centre, true))?;
    let c = meet(p_in, t_in, p_out, t_out).filter(|m| dist(*m, c) <= 3.0 * h).unwrap_or(c);
    let back = (-t_in.0, -t_in.1);
    let cos = (back.0 * t_out.0 + back.1 * t_out.1).clamp(-1.0, 1.0);
    let half = cos.acos() / 2.0;
    // Into the inside of the corner.
    let bi = unit((0.0, 0.0), (back.0 + t_out.0, back.1 + t_out.1))?;
    if !(half > 1e-3 && half.is_finite()) {
        return None;
    }
    // Cut each arm where its rungs are this far back from the corner along its line.
    // (Half a step more, so a rung isn't cast along the inner edge itself.)
    let d = (h / half.tan()).clamp(0.3 * h, 4.0 * h) + STEP_MM / 2.0;
    let ahead = |q: P, dir: P| (q.0 - c.0) * dir.0 + (q.1 - c.1) * dir.1;
    let last = first.centre.iter().rposition(|q| -ahead(*q, t_in) >= d).unwrap_or(0);
    first.keep(last, true);
    let next = second.centre.iter().position(|q| ahead(*q, t_out) >= d).unwrap_or(second.centre.len().saturating_sub(1));
    second.keep(next, false);
    // Near the corner the skeleton bends into it, and normals cast square to it would lie askew:
    // those rungs are cast square to the arm instead.
    first.square(t_in, 2.0 * h, true, segs);
    second.square(t_out, 2.0 * h, false, segs);
    // The inner side is where the second arm turns.
    let inner_left = t_in.0 * t_out.1 - t_in.1 * t_out.0 > 0.0;
    let pick = |a: &Arm, end: bool| -> Option<(P, P, P)> {
        let get = |v: &Vec<P>| if end { v.last().copied() } else { v.first().copied() };
        let (l, r, m) = (get(&a.left)?, get(&a.right)?, get(&a.centre)?);
        Some(if inner_left { (l, r, m) } else { (r, l, m) })
    };
    let (fi, fo, fc) = pick(first, true)?;
    let (si, so, sc) = pick(second, false)?;
    let reach = h / half.sin();
    let toward = |dir: P, cap: f64| -> P {
        let t = ray(segs, c, dir).map_or(cap, |t| t.min(cap));
        (c.0 + dir.0 * t, c.1 + dir.1 * t)
    };
    let inner = toward(bi, reach);
    let out = (-bi.0, -bi.1);
    let (outer_a, outer_b): (Vec<P>, Vec<P>) = if reach <= MITRE_LIMIT * h {
        let m = toward(out, reach);
        (vec![fo, m], vec![m, so])
    } else {
        // Along each outer rail's line to the cap, `lim` out from the corner (or the outline,
        // when it is nearer: a letter's flat point).
        let lim = ray(segs, c, out).map_or(MITRE_LIMIT * h, |t| (t - END_INSET_MM).min(MITRE_LIMIT * h));
        let on_line = |o: P, dir: P| -> P {
            let along = dir.0 * out.0 + dir.1 * out.1;
            let s = if along > 1e-9 { ((lim - ((o.0 - c.0) * out.0 + (o.1 - c.1) * out.1)) / along).max(0.0) } else { 0.0 };
            (o.0 + dir.0 * s, o.1 + dir.1 * s)
        };
        let p1 = on_line(fo, t_in);
        let p2 = on_line(so, (-t_out.0, -t_out.1));
        let mid = ((p1.0 + p2.0) / 2.0, (p1.1 + p2.1) / 2.0);
        (vec![fo, p1, mid], vec![mid, p2, so])
    };
    // Where an arm's last inner point is (nearly) the inner corner, the inner rail stays on the
    // corner while the outer one goes round, so the rungs fan without crossing.
    let fold = |p: P| if dist(p, inner) < 0.3 * h { inner } else { p };
    let (mut cl, mut il, mut ol) = (vec![], vec![], vec![]);
    for (inn, outer, cen) in [(vec![fold(fi), inner], outer_a, vec![fc, c]), (vec![inner, fold(si)], outer_b, vec![c, sc])] {
        // Finite and short (a few half widths), so the cast is exact.
        let n = ((len_of(&outer).max(len_of(&inn)) / STEP_MM).ceil() as usize).clamp(1, 1000) + 1;
        il.extend(spread(&inn, n).into_iter().skip(1));
        ol.extend(spread(&outer, n).into_iter().skip(1));
        cl.extend(spread(&cen, n).into_iter().skip(1));
    }
    // The last rung is the second arm's first: it is left to the arm.
    il.pop();
    ol.pop();
    cl.pop();
    Some(if inner_left { (cl, il, ol) } else { (cl, ol, il) })
}

/// The direction of `pts` leaving its start (`at_start`) or arriving at its end, over about 1 mm.
fn heading(pts: &[P], at_start: bool) -> Option<P> {
    let k = ((1.0 / STEP_MM) as usize).min(pts.len().saturating_sub(1)).max(1);
    if at_start { unit(*pts.first()?, *pts.get(k)?) } else { unit(*pts.get(pts.len().checked_sub(1 + k)?)?, *pts.last()?) }
}

/// `pts` resampled to `n` points, a single point repeated.
fn spread(pts: &[P], n: usize) -> Vec<P> {
    if pts.len() < 2 || len_of(pts) <= 0.0 {
        return pts.first().map_or(vec![], |p| vec![*p; n]);
    }
    resample(pts, n)
}
/// The rungs of centre line `c` (an arm), cast to the outline `segs`; `typical` is the stroke's
/// typical half width, for points the distance transform misses.
fn cast(c: Vec<P>, grid: &Grid, segs: &[(P, P)], typical: f64, column_at: &impl Fn(f64) -> bool) -> Arm {
    let hw: Vec<f64> = c.iter().map(|&q| grid.half_width(q)).collect();
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
    Arm { centre: c, left, right, column }
}

/// Arms `k` and `k + 1`, both mutable.
fn two_arms(arms: &mut [Arm], k: usize) -> Option<(&mut Arm, &mut Arm)> {
    let (head, tail) = arms.split_at_mut_checked(k + 1)?;
    Some((head.last_mut()?, tail.first_mut()?))
}

/// A line to sew: its nodes, its sides between corners and the corners.
type Line = (usize, usize, Vec<Vec<P>>, Vec<P>);

/// The strokes of graph `gr`: centre lines smoothed, free ends extended to the outline, rails
/// cast to the outline; a dot (a node with no strokes) becomes a short stroke across it. A point
/// is part of a satin column when `column_at` says so of its width as drawn, rail to rail (mm).
/// A stroke with corners ([`skeleton::Edge::corners`]) is smoothed a side at a time and turns
/// each corner as one column ([`corner`]).
pub(crate) fn strokes(gr: &Graph, grid: &Grid, segs: &[(P, P)], column_at: impl Fn(f64) -> bool) -> Vec<Stroke> {
    let mut out = vec![];
    let mut next_node = gr.nodes.len();
    let mut lines: Vec<Line> = vec![];
    for (_, e) in gr.live() {
        let mut cuts: Vec<usize> = e.corners.iter().filter_map(|c| index_of(&e.pts, *c)).filter(|&i| i > 0 && i + 1 < e.pts.len()).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut bounds = vec![0];
        bounds.extend(&cuts);
        bounds.push(e.pts.len().saturating_sub(1));
        let mut arms: Vec<Vec<P>> = bounds.windows(2).filter_map(|w| if let [a, b] = w { e.pts.get(*a..=*b).map(smooth) } else { None }).collect();
        let mut corners: Vec<P> = cuts.iter().filter_map(|&i| e.pts.get(i).copied()).collect();
        if arms.iter().any(|a| a.len() < 2) || corners.len() + 1 != arms.len() {
            // A side too short to smooth: one line, without turning the corners.
            arms = vec![smooth(&e.pts)];
            corners.clear();
        }
        if arms.first().is_none_or(|a| a.len() < 2) {
            continue;
        }
        let reach = |n: usize| gr.nodes.get(n).map_or(0.0, |x| x.r) * 3.0 + 0.5;
        let h = median(e.pts.iter().map(|q| grid.half_width(*q)).collect());
        if e.a != e.b
            && gr.degree(e.b) == 1
            && let Some(last) = arms.last_mut()
        {
            extend_end(last, segs, reach(e.b), h);
        }
        if e.a != e.b
            && gr.degree(e.a) == 1
            && let Some(first) = arms.first_mut()
        {
            first.reverse();
            extend_end(first, segs, reach(e.a), h);
            first.reverse();
        }
        lines.push((e.a, e.b, arms, corners));
    }
    for (i, n) in gr.nodes.iter().enumerate() {
        if !n.joined && gr.degree(i) == 0 {
            let r = n.r.max(grid.px);
            let (a, b) = ((n.pos.0 - r, n.pos.1), (n.pos.0 + r, n.pos.1));
            let k = ((2.0 * r / STEP_MM).ceil() as usize).clamp(1, 1000);
            let c: Vec<P> = (0..=k).map(|j| (a.0 + (b.0 - a.0) * j as f64 / k as f64, a.1)).collect();
            lines.push((next_node, next_node + 1, vec![c], vec![]));
            next_node += 2;
        }
    }
    for (a, b, arms, corners) in lines {
        let typical = {
            let m = median(arms.iter().flatten().map(|&q| grid.half_width(q)).filter(|&h| h > 0.0).collect());
            if m > 0.0 { m } else { grid.px }
        };
        let mut sides: Vec<Arm> = arms.into_iter().map(|c| cast(c, grid, segs, typical, &column_at)).collect();
        let mut turns: Vec<Option<Turn>> = vec![];
        for (k, c) in corners.iter().enumerate() {
            turns.push(two_arms(&mut sides, k).and_then(|(first, second)| corner(first, second, *c, segs)));
        }
        let mut s = assemble(a, b, sides, &turns);
        // A rung turned or cast again at a corner is a column only where it is wide enough too.
        for ((flag, l), r) in s.column.iter_mut().zip(&s.left).zip(&s.right) {
            *flag = *flag && column_at(dist(*l, *r));
        }
        out.push(s);
    }
    out.retain(|s| s.centre.len() >= 2);
    trim_branches(&mut out, gr);
    out
}

/// The stroke from node `a` to `b` made of `sides` with the rungs of `turns` between them.
fn assemble(a: usize, b: usize, sides: Vec<Arm>, turns: &[Option<Turn>]) -> Stroke {
    let mut s = Stroke { a, b, centre: vec![], left: vec![], right: vec![], column: vec![], bend: vec![] };
    for (k, arm) in sides.into_iter().enumerate() {
        if k > 0 {
            let flag = s.column.last().copied().unwrap_or(false) && arm.column.first().copied().unwrap_or(false);
            match turns.get(k - 1) {
                Some(Some((cc, cl, cr))) => {
                    // The corner's own point is the middle rung's centre.
                    let mid = cc.len() / 2;
                    for (j, ((q, l), r)) in cc.iter().zip(cl).zip(cr).enumerate() {
                        s.centre.push(*q);
                        s.left.push(*l);
                        s.right.push(*r);
                        s.column.push(flag);
                        s.bend.push(j == mid);
                    }
                }
                // No room to turn it: still a corner of the centre line.
                _ => {
                    if let Some(last) = s.bend.last_mut() {
                        *last = true;
                    }
                }
            }
        }
        let skip = usize::from(k > 0 && s.centre.last() == arm.centre.first());
        let n = arm.centre.len().saturating_sub(skip);
        s.centre.extend(arm.centre.into_iter().skip(skip));
        s.left.extend(arm.left.into_iter().skip(skip));
        s.right.extend(arm.right.into_iter().skip(skip));
        s.column.extend(arm.column.into_iter().skip(skip));
        s.bend.extend(std::iter::repeat_n(false, n));
    }
    s
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
