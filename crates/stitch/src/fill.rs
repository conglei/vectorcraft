//! Tatami fill: a filled region sewn in parallel rows of running stitch.
//!
//! The region is turned so that rows run along `u`; rows sit `spacing` apart in `v`, on a grid
//! anchored at `v = 0` so neighbouring shapes line up. Each row is cut into the spans that lie
//! inside the region by the fill rule. Spans are sewn back and forth, row after row, as long as
//! the short connector from one span's end to the next span's start stays inside the region: that
//! is one section, filled continuously. The next section starts at the first span not yet sewn,
//! in row order, reached by a travel run along the region's edge (inset a little, when the inset
//! point is still inside) when both ends lie on the same outline, by a straight run when that stays
//! inside, and otherwise by a new run, which the plan reaches with a trim and a jump.

use vectorcraft_geom::FillRule;

use crate::{MAX_STEPS, Pt, TOO_MANY_STEPS};

/// The most rows a fill may have.
pub const MAX_ROWS: usize = 100_000;
/// The error for a fill past [`MAX_ROWS`].
pub const TOO_MANY_ROWS: &str = "the fill has too many rows: use a larger spacing or fill a smaller shape";
/// The most (edge, band) pairs the row index may hold.
const MAX_BAND_ENTRIES: usize = 20_000_000;
/// Rounding to plan units moves a point by up to 0.05 mm per axis, so a stitch split to this much
/// under the maximum stays within it once rounded.
pub(crate) const ROUNDING_SLACK_MM: f64 = 0.15;

/// Tatami fill settings. Lengths in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillParams {
    /// Row direction in degrees, counter-clockwise from horizontal as seen on the page.
    pub angle: f64,
    /// Distance between rows.
    pub spacing: f64,
    /// Stitch length along a row.
    pub stitch_length: f64,
    /// How far each row's penetrations move along the row, as a fraction of the stitch length,
    /// so they don't line up from row to row.
    pub stagger: f64,
    /// The longest stitch the machine takes: longer stitches are split evenly.
    pub max_stitch: f64,
}

impl Default for FillParams {
    fn default() -> Self {
        Self { angle: 0.0, spacing: 0.4, stitch_length: 3.0, stagger: 0.25, max_stitch: 12.1 }
    }
}

type P = (f64, f64);

/// A point on an outline: ring, edge index, fraction along the edge.
#[derive(Clone, Copy, Debug)]
struct RingPos {
    ring: usize,
    edge: usize,
    t: f64,
}

#[derive(Clone, Copy)]
struct Edge {
    a: P,
    b: P,
    ring: usize,
    idx: usize,
}

/// A stretch of a row inside the region, `u0 < u1`.
#[derive(Clone, Copy)]
struct Span {
    /// Row number on the global grid (`v = k * spacing`).
    k: i64,
    v: f64,
    u0: f64,
    u1: f64,
    p0: RingPos,
    p1: RingPos,
    done: bool,
}

/// The region in row coordinates, with its edges indexed by band.
pub(crate) struct Region {
    rings: Vec<Vec<P>>,
    /// Cumulative edge lengths per ring: `cum[r][i]` is the length up to vertex `i`.
    cum: Vec<Vec<f64>>,
    edges: Vec<Edge>,
    /// Edge indices per band `[k * spacing, (k + 1) * spacing]`, from `k0`.
    bands: Vec<Vec<u32>>,
    k0: i64,
    spacing: f64,
    rule: FillRule,
}

/// Row direction and row normal for `angle` degrees, in page coordinates (y down).
pub(crate) fn frame(angle: f64) -> (P, P) {
    let a = angle.to_radians();
    ((a.cos(), -a.sin()), (a.sin(), a.cos()))
}

fn to_uv(f: (P, P), p: P) -> P {
    let (d, n) = f;
    (p.0 * d.0 + p.1 * d.1, p.0 * n.0 + p.1 * n.1)
}

fn from_uv(f: (P, P), q: P) -> P {
    let (d, n) = f;
    (q.0 * d.0 + q.1 * n.0, q.0 * d.1 + q.1 * n.1)
}

fn dist(a: P, b: P) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn cross(a: P, b: P) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

impl Region {
    pub(crate) fn new(rings_mm: &[Vec<P>], rule: FillRule, f: (P, P), spacing: f64) -> Result<Option<Region>, String> {
        let mut rings: Vec<Vec<P>> = vec![];
        for ring in rings_mm {
            let mut r: Vec<P> = Vec::with_capacity(ring.len());
            for &p in ring {
                if !(p.0.is_finite() && p.1.is_finite()) {
                    continue;
                }
                let q = to_uv(f, p);
                if r.last().is_none_or(|l| dist(*l, q) > 1e-9) {
                    r.push(q);
                }
            }
            while r.len() > 1 && r.first().zip(r.last()).is_some_and(|(a, b)| dist(*a, *b) <= 1e-9) {
                r.pop();
            }
            if r.len() >= 3 {
                rings.push(r);
            }
        }
        let mut edges = vec![];
        let mut cum = vec![];
        let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
        for (ri, r) in rings.iter().enumerate() {
            let mut c = Vec::with_capacity(r.len() + 1);
            let mut total = 0.0;
            c.push(0.0);
            for (i, &a) in r.iter().enumerate() {
                let b = r.get(i + 1).or(r.first()).copied().unwrap_or(a);
                total += dist(a, b);
                c.push(total);
                vmin = vmin.min(a.1);
                vmax = vmax.max(a.1);
                edges.push(Edge { a, b, ring: ri, idx: i });
            }
            cum.push(c);
        }
        if edges.is_empty() || !(vmin.is_finite() && vmax.is_finite()) {
            return Ok(None);
        }
        let k0 = (vmin / spacing).floor() - 1.0;
        let k1 = (vmax / spacing).floor() + 1.0;
        if !(k0.is_finite() && k1.is_finite()) || k1 - k0 + 1.0 > MAX_ROWS as f64 {
            return Err(TOO_MANY_ROWS.into());
        }
        // Finite and within MAX_ROWS of each other; both fit i64 as the points are bounded.
        let (k0, k1) = (k0 as i64, k1 as i64);
        let nb = usize::try_from(k1 - k0 + 1).map_err(|_| TOO_MANY_ROWS.to_string())?;
        let mut bands: Vec<Vec<u32>> = vec![vec![]; nb];
        let mut entries = 0usize;
        for (ei, e) in edges.iter().enumerate() {
            let lo = e.a.1.min(e.b.1);
            let hi = e.a.1.max(e.b.1);
            // One band of slack each side absorbs float noise at band boundaries.
            let b0 = ((lo / spacing).floor() as i64 - 1 - k0).max(0);
            let b1 = ((hi / spacing).floor() as i64 + 1 - k0).min(k1 - k0);
            let ei = u32::try_from(ei).map_err(|_| TOO_MANY_STEPS.to_string())?;
            for b in b0..=b1 {
                entries += 1;
                if entries > MAX_BAND_ENTRIES {
                    return Err(TOO_MANY_ROWS.into());
                }
                if let Some(list) = usize::try_from(b).ok().and_then(|b| bands.get_mut(b)) {
                    list.push(ei);
                }
            }
        }
        Ok(Some(Region { rings, cum, edges, bands, k0, spacing, rule }))
    }

    fn inside_winding(&self, w: i64) -> bool {
        match self.rule {
            FillRule::NonZero => w != 0,
            FillRule::EvenOdd => w % 2 != 0,
        }
    }

    /// The edges near `v`.
    fn band(&self, v: f64) -> impl Iterator<Item = &Edge> {
        let k = (v / self.spacing).floor();
        let idx = if k.is_finite() { usize::try_from(k as i64 - self.k0).ok() } else { None };
        idx.and_then(|i| self.bands.get(i)).into_iter().flatten().filter_map(|&e| self.edges.get(e as usize))
    }

    /// Whether `q` lies inside the region or on its outline.
    pub(crate) fn inside(&self, q: P) -> bool {
        let mut w = 0i64;
        for e in self.band(q.1) {
            if seg_dist(q, e.a, e.b) < 1e-6 {
                return true;
            }
            if (e.a.1 <= q.1) != (e.b.1 <= q.1) {
                let t = (q.1 - e.a.1) / (e.b.1 - e.a.1);
                let u = e.a.0 + t * (e.b.0 - e.a.0);
                if u > q.0 {
                    w += if e.b.1 > e.a.1 { 1 } else { -1 };
                }
            }
        }
        self.inside_winding(w)
    }

    /// Whether the straight run `p`–`q` stays inside: it crosses no edge and its middle is inside.
    pub(crate) fn clear(&self, p: P, q: P) -> bool {
        let r = (q.0 - p.0, q.1 - p.1);
        let (lo, hi) = (p.1.min(q.1), p.1.max(q.1));
        let mut v = (lo / self.spacing).floor() * self.spacing;
        loop {
            for e in self.band(v + self.spacing * 0.5) {
                let s = (e.b.0 - e.a.0, e.b.1 - e.a.1);
                let den = cross(r, s);
                if den.abs() < 1e-12 {
                    continue;
                }
                let ap = (e.a.0 - p.0, e.a.1 - p.1);
                let t = cross(ap, s) / den;
                let w = cross(ap, r) / den;
                if t > 1e-6 && t < 1.0 - 1e-6 && (-1e-9..=1.0 + 1e-9).contains(&w) {
                    return false;
                }
            }
            v += self.spacing;
            if v > hi || !v.is_finite() {
                break;
            }
        }
        self.inside(lerp(p, q, 0.5))
    }

    fn point(&self, at: RingPos) -> P {
        let Some(r) = self.rings.get(at.ring) else { return (0.0, 0.0) };
        let a = r.get(at.edge).copied().unwrap_or((0.0, 0.0));
        let b = r.get(at.edge + 1).or(r.first()).copied().unwrap_or(a);
        lerp(a, b, at.t)
    }

    /// The spans of row `k`, left to right.
    fn spans(&self, k: i64) -> Vec<Span> {
        let v = k as f64 * self.spacing;
        let mut xs: Vec<(f64, i64, RingPos)> = vec![];
        for e in self.band(v) {
            if (e.a.1 <= v) != (e.b.1 <= v) {
                let t = (v - e.a.1) / (e.b.1 - e.a.1);
                let u = e.a.0 + t * (e.b.0 - e.a.0);
                xs.push((u, if e.b.1 > e.a.1 { 1 } else { -1 }, RingPos { ring: e.ring, edge: e.idx, t }));
            }
        }
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = vec![];
        let mut w = 0i64;
        let mut open: Option<(f64, RingPos)> = None;
        for (u, dir, pos) in xs {
            let was = self.inside_winding(w);
            w += dir;
            let now = self.inside_winding(w);
            if !was && now {
                open = Some((u, pos));
            } else if was
                && !now
                && let Some((u0, p0)) = open.take()
                && u - u0 > 1e-9
            {
                out.push(Span { k, v, u0, u1: u, p0, p1: pos, done: false });
            }
        }
        out
    }

    /// Travel along ring `a.ring` from `a` to `b` by the shorter way, inner vertices inset by
    /// `inset` where the inset point is inside. Both ends are included.
    fn ring_travel(&self, a: RingPos, b: RingPos, inset: f64) -> Vec<P> {
        let (Some(r), Some(c)) = (self.rings.get(a.ring), self.cum.get(a.ring)) else { return vec![] };
        let n = r.len();
        let total = c.last().copied().unwrap_or(0.0);
        let at = |p: RingPos| {
            let l0 = c.get(p.edge).copied().unwrap_or(0.0);
            let l1 = c.get(p.edge + 1).copied().unwrap_or(l0);
            l0 + (l1 - l0) * p.t
        };
        let (sa, sb) = (at(a), at(b));
        let fwd = (sb - sa).rem_euclid(total.max(1e-12));
        let mut out = vec![self.point(a)];
        let mut verts: Vec<usize> = vec![];
        if fwd <= total - fwd {
            let mut count = (b.edge + n - a.edge) % n;
            if a.edge == b.edge && b.t < a.t {
                count = n;
            }
            for j in 1..=count {
                verts.push((a.edge + j) % n);
            }
        } else {
            let mut count = (a.edge + n - b.edge) % n;
            if a.edge == b.edge && b.t > a.t {
                count = n;
            }
            for j in 0..count {
                verts.push((a.edge + n - j) % n);
            }
        }
        for j in verts {
            let Some(&v) = r.get(j) else { continue };
            let prev = r.get((j + n - 1) % n).copied().unwrap_or(v);
            let next = r.get((j + 1) % n).copied().unwrap_or(v);
            out.push(self.inset(prev, v, next, inset));
        }
        out.push(self.point(b));
        out
    }

    /// Vertex `v` moved `by` along its bisector, to whichever side is inside; `v` if neither is.
    fn inset(&self, prev: P, v: P, next: P, by: f64) -> P {
        let n1 = unit_normal(prev, v);
        let n2 = unit_normal(v, next);
        let m = (n1.0 + n2.0, n1.1 + n2.1);
        let len = m.0.hypot(m.1);
        if len < 1e-9 || by <= 0.0 {
            return v;
        }
        let m = (m.0 / len * by, m.1 / len * by);
        for q in [(v.0 + m.0, v.1 + m.1), (v.0 - m.0, v.1 - m.1)] {
            if self.inside(q) && self.clear(v, q) {
                return q;
            }
        }
        v
    }
}

fn unit_normal(a: P, b: P) -> P {
    let d = (b.0 - a.0, b.1 - a.1);
    let l = d.0.hypot(d.1);
    if l < 1e-12 { (0.0, 0.0) } else { (-d.1 / l, d.0 / l) }
}

pub(crate) fn seg_dist(q: P, a: P, b: P) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let l2 = d.0 * d.0 + d.1 * d.1;
    let t = if l2 > 0.0 { (((q.0 - a.0) * d.0 + (q.1 - a.1) * d.1) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(q, lerp(a, b, t))
}

/// Points from `a` (exclusive) to `b` (inclusive), no step longer than `step`.
pub(crate) fn straight(out: &mut Vec<P>, a: P, b: P, step: f64) {
    let l = dist(a, b);
    let n = if step > 0.0 && l > step { (l / step).ceil().min(1e6) as usize } else { 1 };
    for i in 1..=n {
        out.push(lerp(a, b, i as f64 / n as f64));
    }
}

/// Every stitch of `run` longer than `max` split evenly.
pub(crate) fn split_long(run: &[P], max: f64) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(run.len());
    for &p in run {
        match out.last().copied() {
            Some(a) => straight(&mut out, a, p, max),
            None => out.push(p),
        }
    }
    out
}

/// Runs in millimetres to runs of plan points, repeats dropped. More than `budget` points is an
/// error.
pub(crate) fn to_plan(runs: Vec<Vec<P>>, budget: usize) -> Result<Vec<Vec<Pt>>, String> {
    let mut total = 0usize;
    let mut out = vec![];
    for run in runs {
        let mut pts: Vec<Pt> = Vec::with_capacity(run.len());
        for (x, y) in run {
            let p = Pt::from_mm(x, y).ok_or("the art lies too far from the origin to stitch")?;
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        total = total.saturating_add(pts.len());
        if total > budget {
            return Err(TOO_MANY_STEPS.into());
        }
        if pts.len() > 1 {
            out.push(pts);
        }
    }
    Ok(out)
}

/// Tatami fill of the region the closed `rings` (millimetres, page coordinates) enclose by `rule`.
/// Returns runs of needle points in sewing order; the plan joins them with a trim and a jump.
/// More than `budget` points, or than [`MAX_ROWS`] rows, is an error.
pub fn tatami(rings: &[Vec<(f64, f64)>], rule: FillRule, p: &FillParams, budget: usize) -> Result<Vec<Vec<Pt>>, String> {
    let budget = budget.min(MAX_STEPS);
    let ok = |v: f64, lo: f64| v.is_finite() && v >= lo;
    if !(p.angle.is_finite() && ok(p.spacing, 0.05) && ok(p.stitch_length, 0.1) && ok(p.max_stitch, 0.5) && p.stagger.is_finite()) {
        return Err("fill settings must be finite and positive".into());
    }
    let f = frame(p.angle);
    let Some(mut region) = Region::new(rings, rule, f, p.spacing)? else { return Ok(vec![]) };
    let len = p.stitch_length;
    let max = (p.max_stitch - ROUNDING_SLACK_MM).max(0.3);
    let inset = p.spacing * 0.5;
    // The spans of every row, top to bottom.
    let mut rows: Vec<Vec<Span>> = Vec::with_capacity(region.bands.len());
    let mut count = 0usize;
    for b in 0..region.bands.len() {
        let spans = region.spans(region.k0 + b as i64);
        count = count.saturating_add(spans.len().saturating_mul(2));
        if count > budget {
            return Err(TOO_MANY_STEPS.into());
        }
        rows.push(spans);
    }
    let mut runs: Vec<Vec<P>> = vec![];
    let mut run: Vec<P> = vec![];
    // Where the needle is: the point, the outline position and the row.
    let mut cur: Option<(P, RingPos)> = None;
    // Points in the finished runs.
    let mut held = 0usize;
    let mut cursor = 0usize;
    loop {
        // The first span not yet sewn, in row order.
        while rows.get(cursor).is_some_and(|r| r.iter().all(|s| s.done)) {
            cursor += 1;
        }
        let Some(row) = rows.get_mut(cursor) else { break };
        let Some(span) = row.iter_mut().find(|s| !s.done) else { break };
        span.done = true;
        let span = *span;
        let (a, b) = ((span.u0, span.v), (span.u1, span.v));
        let forward = cur.is_none_or(|(c, _)| dist(c, a) <= dist(c, b));
        let (start, start_pos) = if forward { (a, span.p0) } else { (b, span.p1) };
        if let Some((c, cpos)) = cur {
            if cpos.ring == start_pos.ring {
                let travel = region.ring_travel(cpos, start_pos, inset);
                for w in travel.windows(2) {
                    if let [x, y] = w {
                        straight(&mut run, *x, *y, len);
                    }
                }
            } else if region.clear(c, start) {
                straight(&mut run, c, start, len);
            } else {
                held = held.saturating_add(run.len());
                runs.push(std::mem::take(&mut run));
                run.push(start);
            }
        } else {
            run.push(start);
        }
        let mut sewing = (span, forward);
        loop {
            let (s, fwd) = sewing;
            row_points(&mut run, &s, fwd, len, p.stagger);
            let end = if fwd { ((s.u1, s.v), s.p1) } else { ((s.u0, s.v), s.p0) };
            cur = Some(end);
            // Travel and connectors may add points the final count drops as repeats: allow for
            // them, but stop long before memory runs out.
            if held.saturating_add(run.len()) > budget.saturating_mul(4) {
                return Err(TOO_MANY_STEPS.into());
            }
            // The next row's unsewn span that overlaps this one and is reached by a clear
            // connector, nearest first.
            let next_row = usize::try_from(s.k + 1 - region.k0).ok();
            let Some(next) = next_row.and_then(|r| rows.get_mut(r)) else { break };
            let mut best: Option<(f64, usize, bool)> = None;
            for (i, n) in next.iter().enumerate() {
                if n.done || n.u1 < s.u0 || n.u0 > s.u1 {
                    continue;
                }
                for (fwd, q) in [(true, (n.u0, n.v)), (false, (n.u1, n.v))] {
                    let d = dist(end.0, q);
                    if best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, i, fwd));
                    }
                }
            }
            let Some((_, i, fwd)) = best else { break };
            let Some(n) = next.get_mut(i) else { break };
            let q = if fwd { (n.u0, n.v) } else { (n.u1, n.v) };
            if !region.clear(end.0, q) {
                break;
            }
            n.done = true;
            straight(&mut run, end.0, q, len);
            sewing = (*n, fwd);
        }
    }
    runs.push(run);
    region.edges.clear();
    let runs: Vec<Vec<P>> = runs.into_iter().map(|r| split_long(&r.into_iter().map(|q| from_uv(f, q)).collect::<Vec<_>>(), max)).collect();
    to_plan(runs, budget)
}

/// The needle points of span `s` after its start (already placed), sewn forward (`u0` to `u1`)
/// or back: penetrations on the row's staggered grid, none closer than a quarter stitch to an end.
fn row_points(out: &mut Vec<P>, s: &Span, forward: bool, len: f64, stagger: f64) {
    let phase = ((s.k as f64) * stagger).rem_euclid(1.0) * len;
    let gap = len * 0.25;
    let first = ((s.u0 + gap - phase) / len).ceil();
    let last = ((s.u1 - gap - phase) / len).floor();
    let mut inner: Vec<f64> = vec![];
    if first.is_finite() && last.is_finite() && last >= first && last - first < 1e6 {
        let mut m = first;
        while m <= last {
            inner.push(phase + m * len);
            m += 1.0;
        }
    }
    if forward {
        out.extend(inner.iter().map(|&u| (u, s.v)));
        out.push((s.u1, s.v));
    } else {
        out.extend(inner.iter().rev().map(|&u| (u, s.v)));
        out.push((s.u0, s.v));
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<P> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    fn mm(p: Pt) -> P {
        p.to_mm()
    }

    /// Every stitch segment of `runs` sampled at `n` points.
    fn samples(runs: &[Vec<Pt>], n: usize) -> Vec<P> {
        let mut out = vec![];
        for r in runs {
            for w in r.windows(2) {
                for i in 0..=n {
                    out.push(lerp(mm(w[0]), mm(w[1]), i as f64 / n as f64));
                }
            }
        }
        out
    }

    fn longest(runs: &[Vec<Pt>]) -> f64 {
        runs.iter().flat_map(|r| r.windows(2).map(|w| w[0].dist(w[1]) / 10.0)).fold(0.0, f64::max)
    }

    /// The rows sewn at angle 0: distinct y of horizontal stitches of 1 mm or more.
    fn rows_of(runs: &[Vec<Pt>]) -> Vec<i32> {
        let mut ys: Vec<i32> =
            runs.iter().flat_map(|r| r.windows(2)).filter(|w| w[0].y == w[1].y && (w[0].x - w[1].x).abs() >= 10).map(|w| w[0].y).collect();
        ys.sort_unstable();
        ys.dedup();
        ys
    }

    #[test]
    fn a_rectangle_is_one_run_of_rows_at_the_spacing() {
        let runs = tatami(&[rect(0.0, 0.0, 20.0, 10.0)], FillRule::NonZero, &FillParams::default(), MAX_STEPS).unwrap();
        assert_eq!(runs.len(), 1, "filled continuously");
        // Rows at 0, 0.4, … 9.6 mm: a row on the far edge would only sew along it.
        assert_eq!(rows_of(&runs), (0..=24).map(|k| k * 4).collect::<Vec<_>>());
        assert!(longest(&runs) <= 3.0 * 1.25 + 0.15, "{}", longest(&runs));
        // Rows alternate direction: the first ends on the right, the second on the left.
        let r = &runs[0];
        assert_eq!(r[0], Pt { x: 0, y: 0 });
        assert!(r.contains(&Pt { x: 200, y: 0 }) && r.contains(&Pt { x: 200, y: 4 }));
    }

    #[test]
    fn penetrations_are_staggered_from_row_to_row() {
        let runs = tatami(&[rect(0.0, 0.0, 30.0, 2.0)], FillRule::NonZero, &FillParams::default(), MAX_STEPS).unwrap();
        let xs = |y: i32| -> Vec<i32> { runs[0].iter().filter(|p| p.y == y && p.x > 0 && p.x < 300).map(|p| p.x).collect() };
        let (a, b) = (xs(0), xs(4));
        assert!(!a.is_empty() && !b.is_empty());
        assert!(a.iter().all(|x| !b.contains(x)), "{a:?} {b:?}");
    }

    #[test]
    fn a_ring_is_filled_around_its_hole() {
        let rings = [rect(0.0, 0.0, 30.0, 30.0), rect(10.0, 10.0, 20.0, 20.0)];
        for rule in [FillRule::EvenOdd, FillRule::NonZero] {
            // Non-zero needs the hole wound the other way.
            let mut rings = rings.to_vec();
            if rule == FillRule::NonZero {
                rings[1].reverse();
            }
            let runs = tatami(&rings, rule, &FillParams::default(), MAX_STEPS).unwrap();
            for q in samples(&runs, 8) {
                assert!(!(q.0 > 10.15 && q.0 < 19.85 && q.1 > 10.15 && q.1 < 19.85), "{rule:?}: {q:?} is in the hole");
            }
            assert_eq!(rows_of(&runs).len(), 75, "{rule:?}");
        }
        // Wound the same way, non-zero fills the hole.
        let runs = tatami(&rings, FillRule::NonZero, &FillParams::default(), MAX_STEPS).unwrap();
        assert!(runs.iter().flatten().any(|p| p.y == 148 && p.x > 100 && p.x < 200));
    }

    #[test]
    fn a_concave_shape_is_filled_in_sections_joined_inside_it() {
        // A U: two arms 10 mm wide joined at the bottom.
        let u = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 20.0), (20.0, 20.0), (20.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)];
        let runs = tatami(&[u], FillRule::NonZero, &FillParams::default(), MAX_STEPS).unwrap();
        // Travel along the edge keeps it to one run.
        assert_eq!(runs.len(), 1);
        for q in samples(&runs, 8) {
            assert!(!(q.0 > 10.15 && q.0 < 19.85 && q.1 < 19.85), "{q:?} crosses the gap");
        }
        assert_eq!(rows_of(&runs).len(), 75);
    }

    #[test]
    fn an_angle_turns_the_rows() {
        let p = FillParams { angle: 90.0, ..FillParams::default() };
        let runs = tatami(&[rect(0.0, 0.0, 10.0, 20.0)], FillRule::NonZero, &p, MAX_STEPS).unwrap();
        let mut xs: Vec<i32> = runs.iter().flatten().map(|p| p.x).collect();
        xs.sort_unstable();
        xs.dedup();
        assert_eq!(xs.len(), 25, "vertical rows across 10 mm");
    }

    #[test]
    fn too_many_rows_or_points_are_errors() {
        let p = FillParams { spacing: 0.05, ..FillParams::default() };
        assert_eq!(tatami(&[rect(0.0, 0.0, 10.0, 10_000.0)], FillRule::NonZero, &p, MAX_STEPS).unwrap_err(), TOO_MANY_ROWS);
        assert_eq!(tatami(&[rect(0.0, 0.0, 100.0, 100.0)], FillRule::NonZero, &FillParams::default(), 100).unwrap_err(), TOO_MANY_STEPS);
        assert!(tatami(&[rect(0.0, 0.0, 1.0, 1.0)], FillRule::NonZero, &FillParams { spacing: f64::NAN, ..FillParams::default() }, 100).is_err());
        assert!(tatami(&[vec![(0.0, 0.0), (1.0, 1.0)]], FillRule::NonZero, &FillParams::default(), 100).unwrap().is_empty());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// A ring (a rectangle with a rectangular hole): every needle point inside the shape grown
        /// by the spacing, nothing in the hole, no stitch past the maximum, rows at the spacing.
        #[test]
        fn fill_stays_in_the_shape(
            w in 8.0f64..60.0, h in 8.0f64..60.0, hole in 0.1f64..0.7,
            angle in -180.0f64..180.0, spacing in 0.3f64..1.5, len in 1.0f64..7.0,
            stagger in 0.0f64..1.0, max in 2.0f64..12.1,
        ) {
            let (hx0, hy0) = (w * (0.5 - hole / 2.0), h * (0.5 - hole / 2.0));
            let (hx1, hy1) = (w - hx0, h - hy0);
            let rings = [rect(0.0, 0.0, w, h), rect(hx0, hy0, hx1, hy1)];
            let p = FillParams { angle, spacing, stitch_length: len, stagger, max_stitch: max };
            let runs = tatami(&rings, FillRule::EvenOdd, &p, MAX_STEPS).unwrap();
            prop_assert!(!runs.is_empty());
            let tol = spacing + 0.1;
            for r in &runs {
                for q in r.iter().map(|p| mm(*p)) {
                    prop_assert!(q.0 > -tol && q.0 < w + tol && q.1 > -tol && q.1 < h + tol, "{:?}", q);
                }
            }
            for q in samples(&runs, 6) {
                prop_assert!(!(q.0 > hx0 + 0.2 && q.0 < hx1 - 0.2 && q.1 > hy0 + 0.2 && q.1 < hy1 - 0.2), "{:?} in the hole", q);
            }
            prop_assert!(longest(&runs) <= max + 1e-9, "{} > {}", longest(&runs), max);
        }

        /// On a rectangle at angle 0 the rows are the requested spacing apart.
        #[test]
        fn rows_keep_the_spacing(h in 5.0f64..50.0, spacing in 0.3f64..2.0) {
            let p = FillParams { spacing, ..FillParams::default() };
            let runs = tatami(&[rect(0.0, 0.0, 20.0, h)], FillRule::NonZero, &p, MAX_STEPS).unwrap();
            let rows = rows_of(&runs);
            let want = (h / spacing).floor() + 1.0;
            prop_assert!((rows.len() as f64 - want).abs() <= 1.0, "{} rows, want {}", rows.len(), want);
            let mean = (rows.last().unwrap() - rows[0]) as f64 / 10.0 / (rows.len() - 1) as f64;
            prop_assert!((mean - spacing).abs() <= 0.06, "mean spacing {} for {}", mean, spacing);
        }
    }
}
