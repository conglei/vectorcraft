//! A glyph's skeleton: the outline rasterised, its exact Euclidean distance transform
//! (Felzenszwalb and Huttenlocher, "Distance Transforms of Sampled Functions", 2012) and its
//! Zhang–Suen thinning (Zhang and Suen, "A fast parallel algorithm for thinning digital
//! patterns", 1984), turned into a graph of strokes: end points, junctions and the centre lines
//! between them, with the stroke's half width at each skeleton pixel.

use crate::lettering::P;

/// The longest side of the grid, in pixels: a letter of any size is at most this many pixels
/// across, so the work per glyph is bounded.
const MAX_SIDE: usize = 800;
/// The finest pixel, in mm.
const MIN_PIXEL_MM: f64 = 0.05;
/// Empty pixels round the outline's box, so every row and column starts and ends outside.
const PAD: usize = 2;
/// The most outline segments a glyph may have (hostile input is cut off with an error).
pub(crate) const MAX_SEGMENTS: usize = 20_000;
/// Thinning passes before it gives up (each pass peels one layer; letters need far fewer).
const MAX_PASSES: usize = MAX_SIDE;

/// The rasterised glyph.
pub(crate) struct Grid {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) x0: f64,
    pub(crate) y0: f64,
    /// Pixel size, mm.
    pub(crate) px: f64,
    pub(crate) inside: Vec<bool>,
    /// Distance from each pixel centre to the nearest outside pixel centre, in pixels (0 outside).
    pub(crate) dist: Vec<f64>,
}

impl Grid {
    fn at(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    /// The centre of pixel (`x`, `y`) in mm.
    pub(crate) fn centre(&self, x: usize, y: usize) -> P {
        (self.x0 + (x as f64 + 0.5) * self.px, self.y0 + (y as f64 + 0.5) * self.px)
    }

    /// The stroke's half width at `p` (mm): the distance transform of the pixel holding it, less
    /// half a pixel; 0 outside the grid or outside the glyph.
    pub(crate) fn half_width(&self, p: P) -> f64 {
        let fx = ((p.0 - self.x0) / self.px).floor();
        let fy = ((p.1 - self.y0) / self.px).floor();
        if !(fx.is_finite() && fy.is_finite()) || fx < 0.0 || fy < 0.0 || fx >= self.w as f64 || fy >= self.h as f64 {
            return 0.0;
        }
        // In range, so the casts are exact.
        let i = self.at(fx as usize, fy as usize);
        self.dist.get(i).map_or(0.0, |d| ((d - 0.5) * self.px).max(0.0))
    }
}

/// The outline's segments, closing each ring.
pub(crate) fn segments(rings: &[Vec<P>]) -> Vec<(P, P)> {
    let mut out = vec![];
    for r in rings {
        for (i, &a) in r.iter().enumerate() {
            let Some(&b) = r.get((i + 1) % r.len()) else { continue };
            if a != b {
                out.push((a, b));
            }
        }
    }
    out
}

/// The glyph rasterised by the nonzero rule, with its distance transform. `None` for an outline
/// of no area.
pub(crate) fn rasterize(segs: &[(P, P)]) -> Option<Grid> {
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for &(a, _) in segs {
        lo = (lo.0.min(a.0), lo.1.min(a.1));
        hi = (hi.0.max(a.0), hi.1.max(a.1));
    }
    let size = (hi.0 - lo.0).max(hi.1 - lo.1);
    if !(size.is_finite() && size > 0.0) {
        return None;
    }
    let px = (size / (MAX_SIDE - 2 * PAD) as f64).max(MIN_PIXEL_MM);
    // At most MAX_SIDE each, so the casts are exact.
    let w = (((hi.0 - lo.0) / px).ceil() as usize).min(MAX_SIDE) + 2 * PAD;
    let h = (((hi.1 - lo.1) / px).ceil() as usize).min(MAX_SIDE) + 2 * PAD;
    let (x0, y0) = (lo.0 - PAD as f64 * px, lo.1 - PAD as f64 * px);
    let mut inside = vec![false; w * h];
    let mut xs: Vec<(f64, i32)> = Vec::new();
    for y in 0..h {
        let yc = y0 + (y as f64 + 0.5) * px;
        xs.clear();
        for &(a, b) in segs {
            if (a.1 <= yc) != (b.1 <= yc) {
                let x = a.0 + (yc - a.1) * (b.0 - a.0) / (b.1 - a.1);
                xs.push((x, if b.1 > a.1 { 1 } else { -1 }));
            }
        }
        xs.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut wind = 0;
        for k in 0..xs.len() {
            let Some(&(xa, d)) = xs.get(k) else { break };
            wind += d;
            let Some(&(xb, _)) = xs.get(k + 1) else { break };
            if wind == 0 {
                continue;
            }
            let from = ((xa - x0) / px - 0.5).ceil().max(0.0);
            let to = ((xb - x0) / px - 0.5).floor().min(w as f64 - 1.0);
            if from > to {
                continue;
            }
            // Clamped to the grid, so the casts are exact.
            for x in from as usize..=to as usize {
                if let Some(c) = inside.get_mut(y * w + x) {
                    *c = true;
                }
            }
        }
    }
    if !inside.contains(&true) {
        return None;
    }
    let dist = distance(&inside, w, h);
    Some(Grid { w, h, x0, y0, px, inside, dist })
}

/// Far, but finite, so the parabola arithmetic stays finite.
const FAR: f64 = 1e20;

/// The one-dimensional squared distance transform of `f` into `d` (Felzenszwalb–Huttenlocher).
fn dt1(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let fv = |q: usize| f.get(q).copied().unwrap_or(FAR);
    let mut k = 0usize;
    if let Some(v0) = v.first_mut() {
        *v0 = 0;
    }
    if let Some(z0) = z.first_mut() {
        *z0 = f64::NEG_INFINITY;
    }
    if let Some(z1) = z.get_mut(1) {
        *z1 = f64::INFINITY;
    }
    for q in 1..n {
        let qf = q as f64;
        let s = loop {
            let vk = v.get(k).copied().unwrap_or(0);
            let vf = vk as f64;
            let s = ((fv(q) + qf * qf) - (fv(vk) + vf * vf)) / (2.0 * qf - 2.0 * vf);
            if k > 0 && z.get(k).is_some_and(|&zk| s <= zk) {
                k -= 1;
                continue;
            }
            break s;
        };
        k += 1;
        if let Some(vk) = v.get_mut(k) {
            *vk = q;
        }
        if let Some(zk) = z.get_mut(k) {
            *zk = s;
        }
        if let Some(zk) = z.get_mut(k + 1) {
            *zk = f64::INFINITY;
        }
    }
    let mut k = 0usize;
    for q in 0..n {
        let qf = q as f64;
        while z.get(k + 1).is_some_and(|&zn| zn < qf) {
            k += 1;
        }
        let vk = v.get(k).copied().unwrap_or(0);
        let dq = qf - vk as f64;
        if let Some(out) = d.get_mut(q) {
            *out = dq * dq + fv(vk);
        }
    }
}

/// Distance from each pixel centre to the nearest outside pixel centre, in pixels.
pub(crate) fn distance(inside: &[bool], w: usize, h: usize) -> Vec<f64> {
    let mut g: Vec<f64> = inside.iter().map(|&i| if i { FAR } else { 0.0 }).collect();
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z) = (vec![0.0; n], vec![0.0; n], vec![0usize; n], vec![0.0; n + 1]);
    for x in 0..w {
        for y in 0..h {
            if let (Some(t), Some(s)) = (f.get_mut(y), g.get(y * w + x)) {
                *t = *s;
            }
        }
        dt1(f.get(..h).unwrap_or(&[]), &mut d, &mut v, &mut z);
        for y in 0..h {
            if let (Some(t), Some(s)) = (g.get_mut(y * w + x), d.get(y)) {
                *t = *s;
            }
        }
    }
    for y in 0..h {
        let row = g.get(y * w..(y + 1) * w).map(<[f64]>::to_vec).unwrap_or_default();
        dt1(&row, &mut d, &mut v, &mut z);
        for x in 0..w {
            if let (Some(t), Some(s)) = (g.get_mut(y * w + x), d.get(x)) {
                *t = s.sqrt();
            }
        }
    }
    g
}

/// The eight neighbours' offsets, clockwise from north: p2 … p9 in Zhang–Suen's naming.
const RING: [(i64, i64); 8] = [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)];

/// The one-pixel skeleton of `g`'s inside (Zhang–Suen).
pub(crate) fn thin(g: &Grid) -> Vec<bool> {
    let (w, h) = (g.w, g.h);
    let mut s = g.inside.clone();
    let get = |s: &[bool], x: usize, y: usize, d: (i64, i64)| -> bool {
        let (nx, ny) = (x as i64 + d.0, y as i64 + d.1);
        nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h && s.get(ny as usize * w + nx as usize).copied().unwrap_or(false)
    };
    let mut gone: Vec<usize> = vec![];
    let mut removing = vec![false; w * h];
    for _ in 0..MAX_PASSES {
        let mut changed = false;
        for step in 0..2 {
            gone.clear();
            for y in 1..h.saturating_sub(1) {
                for x in 1..w.saturating_sub(1) {
                    if !s.get(y * w + x).copied().unwrap_or(false) {
                        continue;
                    }
                    let nb: [bool; 8] = RING.map(|d| get(&s, x, y, d));
                    let b = nb.iter().filter(|&&on| on).count();
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    let a = (0..8).filter(|&i| !nb[i] && nb[(i + 1) % 8]).count();
                    if a != 1 {
                        continue;
                    }
                    let [p2, _, p4, _, p6, _, p8, _] = nb;
                    let ok = if step == 0 { !(p2 && p4 && p6) && !(p4 && p6 && p8) } else { !(p2 && p4 && p8) && !(p2 && p6 && p8) };
                    if ok {
                        gone.push(y * w + x);
                    }
                }
            }
            // Thinning erases a 2 × 2 block (a dot) whole: a pixel whose neighbours all go with
            // it stays.
            for &i in &gone {
                if let Some(c) = removing.get_mut(i) {
                    *c = true;
                }
            }
            let mut removed = 0usize;
            for &i in &gone {
                let (x, y) = (i % w, i / w);
                let alone = RING.iter().all(|&d| {
                    let (nx, ny) = (x as i64 + d.0, y as i64 + d.1);
                    !get(&s, x, y, d) || removing.get(ny as usize * w + nx as usize).copied().unwrap_or(false)
                });
                if !alone && let Some(c) = s.get_mut(i) {
                    *c = false;
                    removed += 1;
                }
            }
            for &i in &gone {
                if let Some(c) = removing.get_mut(i) {
                    *c = false;
                }
            }
            changed |= removed > 0;
        }
        if !changed {
            break;
        }
    }
    s
}

/// A skeleton node: an end, a junction, or the start of a loop.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) pos: P,
    /// The stroke's half width there, mm.
    pub(crate) r: f64,
    /// The edges that meet here (a loop is listed once).
    pub(crate) edges: Vec<usize>,
    /// An edge once met here (it may since have been pruned or merged away): not a dot.
    pub(crate) joined: bool,
}

/// A stroke between two nodes: its centre line in mm, from `a` to `b`.
#[derive(Clone, Debug)]
pub(crate) struct Edge {
    pub(crate) a: usize,
    pub(crate) b: usize,
    pub(crate) pts: Vec<P>,
    pub(crate) alive: bool,
}

impl Edge {
    pub(crate) fn length(&self) -> f64 {
        self.pts.windows(2).map(|w| if let [p, q] = w { (q.0 - p.0).hypot(q.1 - p.1) } else { 0.0 }).sum()
    }
}

/// The skeleton graph.
#[derive(Clone, Debug, Default)]
pub(crate) struct Graph {
    pub(crate) nodes: Vec<Node>,
    pub(crate) edges: Vec<Edge>,
}

impl Graph {
    /// How many edge ends meet at node `n` (a loop counts twice).
    pub(crate) fn degree(&self, n: usize) -> usize {
        self.nodes
            .get(n)
            .map_or(0, |node| node.edges.iter().filter_map(|&e| self.edges.get(e)).filter(|e| e.alive).map(|e| if e.a == e.b { 2 } else { 1 }).sum())
    }

    /// The live edges.
    pub(crate) fn live(&self) -> impl Iterator<Item = (usize, &Edge)> {
        self.edges.iter().enumerate().filter(|(_, e)| e.alive)
    }
}

/// The graph of skeleton `s` of grid `g`: nodes at end pixels, at clusters of junction pixels
/// and on each loop with no other node, joined by the pixel paths between them. More than
/// `max_edges` edges is `None`.
pub(crate) fn graph(g: &Grid, s: &[bool], max_edges: usize) -> Option<Graph> {
    let (w, h) = (g.w, g.h);
    let on =
        |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && s.get(y as usize * w + x as usize).copied().unwrap_or(false);
    let nbrs = |i: usize| -> Vec<usize> {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        RING.iter().filter(|d| on(x + d.0, y + d.1)).map(|d| (y + d.1) as usize * w + (x + d.0) as usize).collect()
    };
    let px_pos = |i: usize| g.centre(i % w, i / w);
    let r_of = |i: usize| g.dist.get(i).map_or(0.0, |d| ((d - 0.5) * g.px).max(0.0));
    // Node pixels: ends, junctions and isolated pixels; junction pixels next to each other are
    // one node.
    let mut node_of: Vec<Option<usize>> = vec![None; w * h];
    let mut out = Graph::default();
    for i in 0..w * h {
        if !s.get(i).copied().unwrap_or(false) || node_of.get(i).copied().flatten().is_some() {
            continue;
        }
        let deg = nbrs(i).len();
        if deg == 2 {
            continue;
        }
        let id = out.nodes.len();
        let mut cluster = vec![i];
        if let Some(c) = node_of.get_mut(i) {
            *c = Some(id);
        }
        if deg > 2 {
            let mut k = 0;
            while let Some(&c) = cluster.get(k) {
                k += 1;
                for q in nbrs(c) {
                    if nbrs(q).len() > 2 && node_of.get(q).copied().flatten().is_none() {
                        if let Some(slot) = node_of.get_mut(q) {
                            *slot = Some(id);
                        }
                        cluster.push(q);
                    }
                }
            }
        }
        let n = cluster.len() as f64;
        let (sx, sy) = cluster.iter().map(|&c| px_pos(c)).fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
        let r = cluster.iter().map(|&c| r_of(c)).fold(0.0, f64::max);
        out.nodes.push(Node { pos: (sx / n, sy / n), r, edges: vec![], joined: false });
    }
    let mut used: Vec<bool> = vec![false; w * h];
    let mut pairs: Vec<(usize, usize)> = vec![];
    let add = |out: &mut Graph, a: usize, b: usize, pts: Vec<P>| -> Option<()> {
        if out.edges.len() >= max_edges {
            return None;
        }
        let e = out.edges.len();
        out.edges.push(Edge { a, b, pts, alive: true });
        for n in [a, b] {
            let node = out.nodes.get_mut(n)?;
            if n == a || b != a {
                node.edges.push(e);
            }
            node.joined = true;
        }
        Some(())
    };
    // Trace from every node pixel along each neighbour that isn't in its own node.
    for i in 0..w * h {
        let Some(a) = node_of.get(i).copied().flatten() else { continue };
        for q in nbrs(i) {
            if node_of.get(q).copied().flatten() == Some(a) || used.get(q).copied().unwrap_or(true) {
                continue;
            }
            let start = out.nodes.get(a)?.pos;
            let mut pts = vec![start];
            let (mut prev, mut cur) = (i, q);
            let mut steps = 0usize;
            let end = loop {
                if let Some(b) = node_of.get(cur).copied().flatten() {
                    break b;
                }
                if let Some(u) = used.get_mut(cur) {
                    *u = true;
                }
                pts.push(px_pos(cur));
                // A path pixel has two neighbours: where it came from and where it goes.
                let ns = nbrs(cur);
                let is_node = |n: usize| node_of.get(n).copied().flatten().is_some();
                let next = ns
                    .iter()
                    .copied()
                    .find(|&n| n != prev && !is_node(n) && !used.get(n).copied().unwrap_or(true))
                    .or_else(|| ns.iter().copied().find(|&n| n != prev && is_node(n)));
                steps += 1;
                match next {
                    Some(n) if steps <= w * h => {
                        prev = cur;
                        cur = n;
                    }
                    // A dead end inside a path (thinning leftovers): it ends here.
                    _ => break usize::MAX,
                }
            };
            if end == usize::MAX {
                // Make the dead end a node of its own.
                let id = out.nodes.len();
                let pos = pts.last().copied().unwrap_or(start);
                out.nodes.push(Node { pos, r: r_of(cur), edges: vec![], joined: false });
                add(&mut out, a, id, pts)?;
                continue;
            }
            if end == a && pts.len() < 4 {
                // Back into its own node at once: thinning leftovers, not a loop.
                continue;
            }
            if pts.len() == 1 {
                // Two nodes side by side: one edge between them.
                let key = (a.min(end), a.max(end));
                if a == end || pairs.contains(&key) {
                    continue;
                }
                pairs.push(key);
            }
            pts.push(out.nodes.get(end)?.pos);
            add(&mut out, a, end, pts)?;
        }
    }
    // Loops with no node on them (an O): a node at their first pixel.
    for i in 0..w * h {
        if !s.get(i).copied().unwrap_or(false) || used.get(i).copied().unwrap_or(true) || node_of.get(i).copied().flatten().is_some() {
            continue;
        }
        let id = out.nodes.len();
        out.nodes.push(Node { pos: px_pos(i), r: r_of(i), edges: vec![], joined: false });
        if let Some(u) = used.get_mut(i) {
            *u = true;
        }
        let mut pts = vec![px_pos(i)];
        let (mut prev, mut cur) = (i, nbrs(i).into_iter().next().unwrap_or(i));
        let mut steps = 0usize;
        while cur != i && steps <= w * h {
            steps += 1;
            if let Some(u) = used.get_mut(cur) {
                *u = true;
            }
            pts.push(px_pos(cur));
            let next = nbrs(cur).into_iter().find(|&n| n != prev && (!used.get(n).copied().unwrap_or(true) || n == i));
            let Some(n) = next else { break };
            prev = cur;
            cur = n;
        }
        pts.push(px_pos(i));
        if pts.len() >= 3 {
            add(&mut out, id, id, pts)?;
        }
    }
    Some(out)
}

/// Remove spurs: an edge from an end to a junction shorter than `factor` times the junction's
/// half width (plus a pixel), the forks thinning leaves at a stroke's square end. Then join the
/// two edges left at a node that had three, and at every node where just two meet and the stroke
/// runs straight on (a digital curve's staircase leaves such nodes all round a ring; a corner
/// stays a node, so a column doesn't turn it).
pub(crate) fn prune(gr: &mut Graph, factor: f64, px: f64) {
    prune_spurs(gr, factor, px);
    for n in 0..gr.nodes.len() {
        if runs_on(gr, n, px) {
            merge(gr, n);
        }
    }
}

/// Whether exactly two edges meet at node `n` and leave it in nearly opposite directions (they
/// bend by less than 45 degrees), measured a few pixels out.
fn runs_on(gr: &Graph, n: usize, px: f64) -> bool {
    let Some(node) = gr.nodes.get(n) else { return false };
    let live: Vec<&Edge> = node.edges.iter().filter_map(|&e| gr.edges.get(e)).filter(|e| e.alive).collect();
    let [e1, e2] = live.as_slice() else { return false };
    let reach = (node.r.max(px) * 2.0).max(4.0 * px);
    let away = |e: &Edge| -> Option<P> {
        let pts: Vec<P> = if e.a == n { e.pts.clone() } else { e.pts.iter().rev().copied().collect() };
        let o = *pts.first()?;
        let q = pts.iter().find(|q| (q.0 - o.0).hypot(q.1 - o.1) >= reach).or(pts.last())?;
        let l = (q.0 - o.0).hypot(q.1 - o.1);
        (l > 1e-12).then(|| ((q.0 - o.0) / l, (q.1 - o.1) / l))
    };
    match (away(e1), away(e2)) {
        (Some(a), Some(b)) => a.0 * b.0 + a.1 * b.1 < -std::f64::consts::FRAC_1_SQRT_2,
        _ => false,
    }
}

fn prune_spurs(gr: &mut Graph, factor: f64, px: f64) {
    for _ in 0..gr.edges.len() {
        let mut changed = false;
        for e in 0..gr.edges.len() {
            let Some(edge) = gr.edges.get(e) else { continue };
            if !edge.alive || edge.a == edge.b {
                continue;
            }
            let (da, db) = (gr.degree(edge.a), gr.degree(edge.b));
            let (end, junction) = match (da, db) {
                (1, d) if d >= 3 => (edge.a, edge.b),
                (d, 1) if d >= 3 => (edge.b, edge.a),
                _ => continue,
            };
            let r = gr.nodes.get(junction).map_or(0.0, |n| n.r);
            if edge.length() >= factor * r + px {
                continue;
            }
            if let Some(ed) = gr.edges.get_mut(e) {
                ed.alive = false;
            }
            if let Some(n) = gr.nodes.get_mut(end) {
                n.edges.retain(|&x| x != e);
            }
            if let Some(n) = gr.nodes.get_mut(junction) {
                n.edges.retain(|&x| x != e);
            }
            merge(gr, junction);
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

/// Join the two edges at node `n` into one when exactly two distinct ones (no loop) meet there.
fn merge(gr: &mut Graph, n: usize) {
    let Some(node) = gr.nodes.get(n) else { return };
    let live: Vec<usize> = node.edges.iter().copied().filter(|&e| gr.edges.get(e).is_some_and(|x| x.alive)).collect();
    let [e1, e2] = live.as_slice() else { return };
    let (e1, e2) = (*e1, *e2);
    let (Some(a), Some(b)) = (gr.edges.get(e1).cloned(), gr.edges.get(e2).cloned()) else { return };
    if a.a == a.b || b.a == b.b {
        return;
    }
    // a runs x → n, b runs n → y.
    let (x, mut first) = if a.b == n { (a.a, a.pts) } else { (a.b, a.pts.into_iter().rev().collect()) };
    let (y, second): (usize, Vec<P>) = if b.a == n { (b.b, b.pts) } else { (b.a, b.pts.into_iter().rev().collect()) };
    first.extend(second.into_iter().skip(1));
    if let Some(ed) = gr.edges.get_mut(e2) {
        ed.alive = false;
    }
    if let Some(ed) = gr.edges.get_mut(e1) {
        *ed = Edge { a: x, b: y, pts: first, alive: true };
    }
    if let Some(node) = gr.nodes.get_mut(n) {
        node.edges.clear();
    }
    if let Some(node) = gr.nodes.get_mut(y) {
        for e in &mut node.edges {
            if *e == e2 {
                *e = e1;
            }
        }
        node.edges.dedup();
    }
    if x == y
        && let Some(node) = gr.nodes.get_mut(x)
    {
        let mut seen = vec![];
        node.edges.retain(|e| {
            let keep = !seen.contains(e);
            seen.push(*e);
            keep
        });
    }
}
