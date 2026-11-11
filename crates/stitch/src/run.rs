//! Running stitch: a polyline stepped at a stitch length.

use crate::{MAX_STEPS, Pt, UNITS_PER_MM};

/// Running stitch along `points_mm`, a polyline in millimetres with a corner flag per vertex
/// (`corners`; a missing flag is not a corner). The first vertex is the first needle point. Each
/// stretch between consecutive corners (both ends count as corners) is divided into
/// `ceil(length / stitch_length)` equal steps placed by arc length, so every corner and the last
/// vertex are hit exactly. With `closed`, the first vertex is the final target. `stitch_length`
/// is in plan units (0.1 mm); below 1 it is taken as 1. Zero-length input gives no points, and
/// vertices that aren't finite or lie beyond a plan point's range are left out. At most
/// [`MAX_STEPS`] + 1 points are made, so a caller can tell the limit was passed.
pub fn run_stitches(points_mm: &[(f64, f64)], corners: &[bool], closed: bool, stitch_length: i32) -> Vec<Pt> {
    let mut pts: Vec<(f64, f64)> = Vec::with_capacity(points_mm.len().saturating_add(1));
    let mut corner: Vec<bool> = Vec::with_capacity(points_mm.len().saturating_add(1));
    for (i, &(x, y)) in points_mm.iter().enumerate() {
        if Pt::from_mm(x, y).is_some() {
            pts.push((x, y));
            corner.push(corners.get(i).copied().unwrap_or(false));
        }
    }
    if closed && let Some(&first) = pts.first() {
        pts.push(first);
        corner.push(true);
    }
    // The ends always count as corners.
    if let Some(c) = corner.first_mut() {
        *c = true;
    }
    if let Some(c) = corner.last_mut() {
        *c = true;
    }
    // Every vertex kept is finite and bounded, so the lengths are finite.
    let total: f64 = pts.windows(2).map(seg_len).sum();
    let Some(&first) = pts.first() else { return vec![] };
    if total <= 0.0 {
        return vec![];
    }
    let step_mm = f64::from(stitch_length.max(1)) / UNITS_PER_MM;
    let mut out: Vec<Pt> = Vec::new();
    push(&mut out, first);
    let mut start = 0;
    for end in 1..pts.len() {
        if !corner.get(end).copied().unwrap_or(false) {
            continue;
        }
        let stretch = pts.get(start..=end).unwrap_or(&[]);
        start = end;
        let length: f64 = stretch.windows(2).map(seg_len).sum();
        if length <= 0.0 {
            continue;
        }
        let room = MAX_STEPS.saturating_add(1).saturating_sub(out.len());
        if room == 0 {
            break;
        }
        // Finite and capped at `room`, so the cast is exact.
        let steps = (length / step_mm).ceil().clamp(1.0, room as f64) as usize;
        let mut walked = 0.0;
        let mut seg = 0;
        for k in 1..=steps {
            if k == steps {
                if let Some(&last) = stretch.last() {
                    push(&mut out, last);
                }
                break;
            }
            let target = length * k as f64 / steps as f64;
            // Advance to the segment holding `target`.
            while let Some(w) = stretch.get(seg..seg + 2) {
                let l = seg_len(w);
                if walked + l >= target || seg + 2 >= stretch.len() {
                    let t = if l > 0.0 { ((target - walked) / l).clamp(0.0, 1.0) } else { 0.0 };
                    if let (Some(a), Some(b)) = (w.first(), w.get(1)) {
                        push(&mut out, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
                    }
                    break;
                }
                walked += l;
                seg += 1;
            }
        }
    }
    out
}

fn seg_len(w: &[(f64, f64)]) -> f64 {
    match (w.first(), w.get(1)) {
        (Some(a), Some(b)) => (b.0 - a.0).hypot(b.1 - a.1),
        _ => 0.0,
    }
}

/// Add the needle point at `p` (mm), unless it rounds to the point before it.
fn push(out: &mut Vec<Pt>, p: (f64, f64)) {
    if let Some(q) = Pt::from_mm(p.0, p.1)
        && out.last() != Some(&q)
    {
        out.push(q);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pt;

    #[test]
    fn a_straight_line_is_stepped_evenly_and_ends_exactly() {
        let p = run_stitches(&[(0.0, 0.0), (10.0, 0.0)], &[true, true], false, 25);
        assert_eq!(p, vec![Pt { x: 0, y: 0 }, Pt { x: 25, y: 0 }, Pt { x: 50, y: 0 }, Pt { x: 75, y: 0 }, Pt { x: 100, y: 0 }]);
        let q = run_stitches(&[(0.0, 0.0), (9.0, 0.0)], &[true, true], false, 25);
        assert_eq!(q.last(), Some(&Pt { x: 90, y: 0 }));
        assert_eq!(q.len(), 5);
    }

    #[test]
    fn corners_always_get_a_needle_point() {
        let p = run_stitches(&[(0.0, 0.0), (3.0, 0.0), (3.0, 3.0)], &[true, true, true], false, 25);
        assert!(p.contains(&Pt { x: 30, y: 0 }));
        assert_eq!(p.last(), Some(&Pt { x: 30, y: 30 }));
    }

    #[test]
    fn closed_runs_return_and_empty_input_gives_nothing() {
        let p = run_stitches(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0)], &[true, true, true], true, 25);
        assert_eq!(p.first(), Some(&Pt { x: 0, y: 0 }));
        assert_eq!(p.last(), Some(&Pt { x: 0, y: 0 }));
        assert!(run_stitches(&[], &[], false, 25).is_empty());
        assert!(run_stitches(&[(1.0, 1.0), (1.0, 1.0)], &[true, true], true, 25).is_empty());
        assert!(run_stitches(&[(f64::NAN, 0.0)], &[true], false, 0).is_empty());
    }

    #[test]
    fn rounding_does_not_drift() {
        let p = run_stitches(&[(0.0, 0.0), (1000.03, 0.0)], &[true, true], false, 25);
        assert_eq!(p.last(), Some(&Pt { x: 10000, y: 0 }));
    }
}
