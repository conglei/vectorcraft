//! Satin column: zig-zag stitches between two rails either side of a centre line.
//!
//! The rails are the centre line offset by half the width along its normals, mitred at corners
//! (the mitre held to twice the half width). Stations sit evenly along the centre line, half the
//! spacing apart, and alternate between the rails, so each rail is pierced once per `spacing`
//! (peak to peak). The first and last stations are the ends of the centre line, so the ends are
//! square. A stitch longer than the maximum is split into equal parts, the split points of every
//! other stitch moved half a part, so they don't form a line down the column.

use crate::fill::{ROUNDING_SLACK_MM, to_plan};
use crate::{MAX_STEPS, Pt, TOO_MANY_STEPS};

/// How far a mitred rail corner may lie from the centre line, in half widths.
const MITER_LIMIT: f64 = 2.0;

/// Satin settings, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SatinParams {
    /// Column width, rail to rail.
    pub width: f64,
    /// Distance between two piercings of one rail (peak to peak).
    pub spacing: f64,
    /// The longest stitch the machine takes: longer stitches are split.
    pub max_stitch: f64,
}

impl Default for SatinParams {
    fn default() -> Self {
        Self { width: 3.0, spacing: 0.4, max_stitch: 12.1 }
    }
}

type P = (f64, f64);

fn dist(a: P, b: P) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn normal(a: P, b: P) -> P {
    let l = dist(a, b);
    if l < 1e-12 { (0.0, 0.0) } else { (-(b.1 - a.1) / l, (b.0 - a.0) / l) }
}

/// The two rails of the centre line `pts` (at least two distinct points), half width `h`.
/// `closed`: the last point repeats the first, and the rails meet there with a mitre.
fn rails(pts: &[P], closed: bool, h: f64) -> (Vec<P>, Vec<P>) {
    let segs = pts.len().saturating_sub(1);
    let seg_n = |i: usize| match (pts.get(i), pts.get(i + 1)) {
        (Some(&a), Some(&b)) => normal(a, b),
        _ => (0.0, 0.0),
    };
    let mut left = Vec::with_capacity(pts.len());
    let mut right = Vec::with_capacity(pts.len());
    for (j, &p) in pts.iter().enumerate() {
        let before = if j > 0 {
            Some(j - 1)
        } else if closed {
            segs.checked_sub(1)
        } else {
            None
        };
        let after = if j < segs {
            Some(j)
        } else if closed {
            Some(0)
        } else {
            None
        };
        let off = match (before.map(seg_n), after.map(seg_n)) {
            (Some(a), Some(b)) => {
                let m = (a.0 + b.0, a.1 + b.1);
                let l = m.0.hypot(m.1);
                if l < 1e-9 {
                    (b.0 * h, b.1 * h)
                } else {
                    let m = (m.0 / l, m.1 / l);
                    let cos = m.0 * b.0 + m.1 * b.1;
                    let scale = if cos > 1.0 / MITER_LIMIT { 1.0 / cos } else { MITER_LIMIT };
                    (m.0 * h * scale, m.1 * h * scale)
                }
            }
            (Some(a), None) | (None, Some(a)) => (a.0 * h, a.1 * h),
            (None, None) => (0.0, 0.0),
        };
        left.push((p.0 + off.0, p.1 + off.1));
        right.push((p.0 - off.0, p.1 - off.1));
    }
    (left, right)
}

/// A satin column along the centre line `centre` (millimetres, page coordinates). `closed`: the
/// line returns to its first point. Returns the needle points in sewing order, or nothing for a
/// line of no length. More than `budget` points is an error.
pub fn satin(centre: &[(f64, f64)], closed: bool, p: &SatinParams, budget: usize) -> Result<Vec<Pt>, String> {
    let budget = budget.min(MAX_STEPS);
    let ok = |v: f64, lo: f64| v.is_finite() && v >= lo;
    if !(ok(p.width, 0.05) && ok(p.spacing, 0.05) && ok(p.max_stitch, 0.5)) {
        return Err("satin settings must be finite and positive".into());
    }
    let mut pts: Vec<P> = Vec::with_capacity(centre.len().saturating_add(1));
    for &q in centre {
        if q.0.is_finite() && q.1.is_finite() && pts.last().is_none_or(|l| dist(*l, q) > 1e-9) {
            pts.push(q);
        }
    }
    if closed {
        while pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(a, b)| dist(*a, *b) <= 1e-9) {
            pts.pop();
        }
        if let Some(&f) = pts.first() {
            pts.push(f);
        }
    }
    let total: f64 = pts.windows(2).map(|w| w.first().zip(w.get(1)).map_or(0.0, |(a, b)| dist(*a, *b))).sum();
    if pts.len() < 2 || total <= 0.0 {
        return Ok(vec![]);
    }
    let (left, right) = rails(&pts, closed, p.width / 2.0);
    let half = p.spacing / 2.0;
    let stations = (total / half).ceil();
    if !stations.is_finite() || stations > budget as f64 {
        return Err(TOO_MANY_STEPS.into());
    }
    // Finite and within the budget, so the cast is exact.
    let stations = (stations as usize).max(1);
    let max = (p.max_stitch - ROUNDING_SLACK_MM).max(0.3);
    let mut out: Vec<P> = Vec::with_capacity(stations + 1);
    let (mut seg, mut walked) = (0usize, 0.0);
    for k in 0..=stations {
        let target = total * k as f64 / stations as f64;
        // Advance to the segment holding `target`.
        let mut t = 1.0;
        while let (Some(&a), Some(&b)) = (pts.get(seg), pts.get(seg + 1)) {
            let l = dist(a, b);
            if walked + l >= target || seg + 2 >= pts.len() {
                t = if l > 0.0 { ((target - walked) / l).clamp(0.0, 1.0) } else { 0.0 };
                break;
            }
            walked += l;
            seg += 1;
        }
        let side = if k % 2 == 0 { &left } else { &right };
        let (Some(&a), Some(&b)) = (side.get(seg), side.get(seg + 1)) else { break };
        let q = lerp(a, b, t);
        if let Some(&prev) = out.last() {
            let l = dist(prev, q);
            if l > max {
                let parts = (l / max).ceil().min(1e6);
                // Every other stitch: split points half a part further on.
                let shift = if k % 2 == 0 { 0.0 } else { 0.5 };
                let mut i = 1.0;
                while i < parts + shift {
                    out.push(lerp(prev, q, (i - shift) / parts));
                    i += 1.0;
                }
            }
        }
        out.push(q);
        if out.len() > budget {
            return Err(TOO_MANY_STEPS.into());
        }
    }
    Ok(to_plan(vec![out], budget)?.into_iter().next().unwrap_or_default())
}

/// Two-rail satin settings, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailsParams {
    /// Distance between two piercings of the longer rail (peak to peak).
    pub spacing: f64,
    /// The longest stitch the machine takes: longer stitches are split.
    pub max_stitch: f64,
    /// Pull compensation: each needle point is pushed this far outward along its rung, so the
    /// column sews as wide as drawn once the thread pulls the fabric in.
    pub compensation: f64,
}

impl Default for RailsParams {
    fn default() -> Self {
        Self { spacing: 0.4, max_stitch: 12.1, compensation: 0.0 }
    }
}

/// Running arc length of `pts`, from 0.
fn arc(pts: &[P]) -> Vec<f64> {
    let mut out = Vec::with_capacity(pts.len());
    let mut s = 0.0;
    for (i, &p) in pts.iter().enumerate() {
        if let Some(&q) = i.checked_sub(1).and_then(|j| pts.get(j)) {
            s += dist(q, p);
        }
        out.push(s);
    }
    out
}

/// `pts` resampled to `n` points evenly by arc length (at least two points in).
pub(crate) fn resample(pts: &[P], n: usize) -> Vec<P> {
    let s = arc(pts);
    let total = s.last().copied().unwrap_or(0.0);
    let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return vec![] };
    if n < 2 || total <= 0.0 {
        return vec![first; n.max(1)];
    }
    let mut out = Vec::with_capacity(n);
    let mut seg = 0usize;
    for k in 0..n {
        let target = total * k as f64 / (n - 1) as f64;
        while seg + 2 < pts.len() && s.get(seg + 1).is_some_and(|&v| v < target) {
            seg += 1;
        }
        let (Some(&a), Some(&b), Some(&sa), Some(&sb)) = (pts.get(seg), pts.get(seg + 1), s.get(seg), s.get(seg + 1)) else { break };
        let t = if sb > sa { ((target - sa) / (sb - sa)).clamp(0.0, 1.0) } else { 0.0 };
        out.push(lerp(a, b, t));
    }
    if let Some(l) = out.last_mut() {
        *l = last;
    }
    out
}

/// A satin column between two rails (millimetres, page coordinates), the digitizer's model:
/// `left` and `right` are paired point by point (each pair a rung); rails of different counts are
/// first resampled to the larger count by arc length. Stations sit every half spacing along the
/// longer rail and alternate between the rails, the other rail taken at the same rung position,
/// so the column follows the outline's width as it changes. Each needle point is pushed outward
/// by the compensation; a stitch longer than the maximum is split as [`satin`] splits it. Returns
/// the needle points in sewing order from the first rung to the last, or nothing for rails of no
/// length or with points that aren't finite. More than `budget` points is an error.
pub fn rails_satin(left: &[(f64, f64)], right: &[(f64, f64)], p: &RailsParams, budget: usize) -> Result<Vec<Pt>, String> {
    let budget = budget.min(MAX_STEPS);
    let ok = |v: f64, lo: f64| v.is_finite() && v >= lo;
    if !(ok(p.spacing, 0.05) && ok(p.max_stitch, 0.5) && p.compensation.is_finite() && p.compensation.abs() <= 5.0) {
        return Err("satin settings must be finite and positive".into());
    }
    let finite = |r: &[P]| r.iter().all(|q| q.0.is_finite() && q.1.is_finite());
    if left.len() < 2 || right.len() < 2 || !finite(left) || !finite(right) {
        return Ok(vec![]);
    }
    if left.len().max(right.len()) > budget {
        return Err(TOO_MANY_STEPS.into());
    }
    let (left, right) = if left.len() == right.len() {
        (left.to_vec(), right.to_vec())
    } else {
        let n = left.len().max(right.len());
        (resample(left, n), resample(right, n))
    };
    let (sl, sr) = (arc(&left), arc(&right));
    let (tl, tr) = (sl.last().copied().unwrap_or(0.0), sr.last().copied().unwrap_or(0.0));
    let (guide, total) = if tl >= tr { (&sl, tl) } else { (&sr, tr) };
    if total <= 0.0 {
        return Ok(vec![]);
    }
    let stations = (total / (p.spacing / 2.0)).ceil();
    if !stations.is_finite() || stations > budget as f64 {
        return Err(TOO_MANY_STEPS.into());
    }
    // Finite and within the budget, so the cast is exact.
    let stations = (stations as usize).max(1);
    let max = (p.max_stitch - ROUNDING_SLACK_MM).max(0.3);
    let mut out: Vec<P> = Vec::with_capacity(stations + 1);
    let mut seg = 0usize;
    for k in 0..=stations {
        let target = total * k as f64 / stations as f64;
        while seg + 2 < left.len() && guide.get(seg + 1).is_some_and(|&v| v < target) {
            seg += 1;
        }
        let (Some(&ga), Some(&gb)) = (guide.get(seg), guide.get(seg + 1)) else { break };
        let t = if gb > ga { ((target - ga) / (gb - ga)).clamp(0.0, 1.0) } else { 0.0 };
        let (Some(&la), Some(&lb), Some(&ra), Some(&rb)) = (left.get(seg), left.get(seg + 1), right.get(seg), right.get(seg + 1)) else { break };
        let (a, b) = (lerp(la, lb, t), lerp(ra, rb, t));
        let w = dist(a, b);
        let out_dir = if w > 1e-9 { ((a.0 - b.0) / w, (a.1 - b.1) / w) } else { (0.0, 0.0) };
        // A negative compensation never crosses the rails over.
        let c = p.compensation.max(-w / 2.0);
        let q = if k % 2 == 0 { (a.0 + out_dir.0 * c, a.1 + out_dir.1 * c) } else { (b.0 - out_dir.0 * c, b.1 - out_dir.1 * c) };
        if let Some(&prev) = out.last() {
            let l = dist(prev, q);
            if l > max {
                let parts = (l / max).ceil().min(1e6);
                let shift = if k % 2 == 0 { 0.0 } else { 0.5 };
                let mut i = 1.0;
                while i < parts + shift {
                    out.push(lerp(prev, q, (i - shift) / parts));
                    i += 1.0;
                }
            }
        }
        out.push(q);
        if out.len() > budget {
            return Err(TOO_MANY_STEPS.into());
        }
    }
    Ok(to_plan(vec![out], budget)?.into_iter().next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn a_tapering_column_follows_its_rails() {
        // 10 mm long, 4 mm wide at the start and 1 mm at the end.
        let left = [(0.0, 2.0), (10.0, 0.5)];
        let right = [(0.0, -2.0), (10.0, -0.5)];
        let run = rails_satin(&left, &right, &RailsParams::default(), MAX_STEPS).unwrap();
        assert_eq!(run.first(), Some(&Pt { x: 0, y: 20 }));
        assert!(run[1].y < 0, "the second point is on the right rail");
        let last = *run.last().unwrap();
        assert_eq!(last.x, 100);
        assert_eq!(last.y.abs(), 5);
        // Compensation widens it on both sides.
        let wide = rails_satin(&left, &right, &RailsParams { compensation: 0.2, ..Default::default() }, MAX_STEPS).unwrap();
        assert_eq!(wide[0], Pt { x: 0, y: 22 });
        assert_eq!(wide[1].y, -22);
        // Rails of different counts and reversed rails of no length.
        let more = [(0.0, -2.0), (5.0, -1.25), (10.0, -0.5)];
        assert!(!rails_satin(&left, &more, &RailsParams::default(), MAX_STEPS).unwrap().is_empty());
        assert!(rails_satin(&[(1.0, 1.0), (1.0, 1.0)], &[(1.0, 1.0), (1.0, 1.0)], &RailsParams::default(), MAX_STEPS).unwrap().is_empty());
        assert!(rails_satin(&[(f64::NAN, 1.0), (1.0, 1.0)], &right, &RailsParams::default(), MAX_STEPS).unwrap().is_empty());
        assert!(rails_satin(&left, &right, &RailsParams { spacing: 0.0, ..Default::default() }, MAX_STEPS).is_err());
        assert_eq!(
            rails_satin(&[(0.0, 1.0), (1000.0, 1.0)], &[(0.0, 0.0), (1000.0, 0.0)], &RailsParams::default(), 100).unwrap_err(),
            TOO_MANY_STEPS
        );
    }

    #[test]
    fn a_wide_rails_column_is_split() {
        let p = RailsParams { max_stitch: 3.0, ..Default::default() };
        let run = rails_satin(&[(0.0, 5.0), (10.0, 5.0)], &[(0.0, -5.0), (10.0, -5.0)], &p, MAX_STEPS).unwrap();
        assert!(longest(&run) <= 3.0, "{}", longest(&run));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Random rails never panic, and no stitch passes the maximum.
        #[test]
        fn rails_satin_stays_under_the_maximum(
            l in prop::collection::vec((-50.0f64..50.0, -50.0f64..50.0), 2..12),
            r in prop::collection::vec((-50.0f64..50.0, -50.0f64..50.0), 2..12),
            max in 1.0f64..12.1, comp in -1.0f64..1.0,
        ) {
            let run = rails_satin(&l, &r, &RailsParams { spacing: 0.4, max_stitch: max, compensation: comp }, MAX_STEPS).unwrap();
            prop_assert!(longest(&run) <= max + 1e-9);
        }
    }

    fn longest(run: &[Pt]) -> f64 {
        run.windows(2).map(|w| w[0].dist(w[1]) / 10.0).fold(0.0, f64::max)
    }

    #[test]
    fn a_straight_column_zig_zags_between_its_rails() {
        let run = satin(&[(0.0, 0.0), (10.0, 0.0)], false, &SatinParams { width: 4.0, ..Default::default() }, MAX_STEPS).unwrap();
        // 10 mm at 0.2 mm per station: 51 needle points, alternating rails.
        assert_eq!(run.len(), 51);
        assert!(run.iter().step_by(2).all(|p| p.y == 20));
        assert!(run.iter().skip(1).step_by(2).all(|p| p.y == -20));
        // Square ends: the first and last stitches lie across the ends of the line.
        assert_eq!(run[0].x, 0);
        assert_eq!(run.last().unwrap().x, 100);
        // Each rail is pierced every 0.4 mm.
        assert_eq!(run[2].x - run[0].x, 4);
    }

    #[test]
    fn a_column_wider_than_the_maximum_stitch_is_split() {
        let p = SatinParams { width: 10.0, spacing: 0.4, max_stitch: 4.0 };
        let run = satin(&[(0.0, 0.0), (10.0, 0.0)], false, &p, MAX_STEPS).unwrap();
        assert!(longest(&run) <= 4.0, "{}", longest(&run));
        // Split points of neighbouring stitches don't line up.
        let inner = |i: usize| -> Vec<i32> { run.iter().skip(i).take(3).map(|p| p.y).collect() };
        assert_ne!(inner(1), inner(5));
    }

    #[test]
    fn a_curved_column_stays_between_its_rails() {
        // A half circle of radius 20 mm, width 6 mm.
        let arc: Vec<P> = (0..=60)
            .map(|i| {
                let a = std::f64::consts::PI * f64::from(i) / 60.0;
                (20.0 * a.cos(), 20.0 * a.sin())
            })
            .collect();
        let run = satin(&arc, false, &SatinParams { width: 6.0, ..Default::default() }, MAX_STEPS).unwrap();
        for q in &run {
            let (x, y) = q.to_mm();
            let r = x.hypot(y);
            assert!((r - 20.0).abs() <= 3.0 + 0.1, "{q:?} at {r}");
        }
        assert!(run.len() > 300);
    }

    #[test]
    fn closed_and_degenerate_lines() {
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let run = satin(&sq, true, &SatinParams::default(), MAX_STEPS).unwrap();
        assert!(run.len() > 190);
        assert!(satin(&[(1.0, 1.0), (1.0, 1.0)], false, &SatinParams::default(), MAX_STEPS).unwrap().is_empty());
        assert!(satin(&[(f64::NAN, 1.0)], false, &SatinParams::default(), MAX_STEPS).unwrap().is_empty());
        assert_eq!(satin(&[(0.0, 0.0), (1000.0, 0.0)], false, &SatinParams::default(), 100).unwrap_err(), TOO_MANY_STEPS);
        assert!(satin(&sq, false, &SatinParams { width: f64::INFINITY, ..Default::default() }, 100).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Along an arc, every needle point lies between the rails and no stitch passes the
        /// maximum.
        #[test]
        fn satin_stays_between_the_rails(
            radius in 5.0f64..80.0, sweep in 0.2f64..6.0, width in 0.5f64..9.0,
            spacing in 0.2f64..2.0, max in 2.0f64..12.1,
        ) {
            let width = width.min(radius);
            // Fine enough that the chords stay within 0.01 mm of the arc.
            let n = 400;
            let arc: Vec<P> = (0..=n).map(|i| {
                let a = sweep * f64::from(i) / f64::from(n);
                (radius * a.cos(), radius * a.sin())
            }).collect();
            let run = satin(&arc, false, &SatinParams { width, spacing, max_stitch: max }, MAX_STEPS).unwrap();
            prop_assert!(!run.is_empty());
            for q in &run {
                let (x, y) = q.to_mm();
                prop_assert!((x.hypot(y) - radius).abs() <= width / 2.0 + 0.1, "{:?}", q);
            }
            prop_assert!(longest(&run) <= max + 1e-9, "{} > {}", longest(&run), max);
        }
    }
}
