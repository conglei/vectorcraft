//! Travel within one thread: the needle carried from where one run ends to where the next starts by
//! running stitch instead of a trim and a jump, where something sewn later hides it. Photo designs
//! (`travelMm`), lettering's short joins and auto-routing ([`vectorcraft_doc::StitchProps::route`])
//! all join this way.

use vectorcraft_geom::FillRule;

use crate::Pt;

/// A region: rings in millimetres (each closed implicitly) and the rule that fills them.
pub type Region<'a> = (&'a [Vec<(f64, f64)>], FillRule);

/// How often the way is checked, mm.
const STEP_MM: f64 = 0.25;
/// The most points a way is checked at (a way longer than 100 mm is checked more sparsely).
const MAX_CHECKS: f64 = 400.0;

/// The winding number of `rings` (mm) around `p`.
pub fn winding(rings: &[Vec<(f64, f64)>], p: (f64, f64)) -> i32 {
    let mut w = 0;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { continue };
            let cross = (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1);
            if a.1 <= p.1 {
                if b.1 > p.1 && cross > 0.0 {
                    w += 1;
                }
            } else if b.1 <= p.1 && cross < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

/// Whether `p` (mm) is inside `rings` by `rule`.
pub fn inside(rings: &[Vec<(f64, f64)>], rule: FillRule, p: (f64, f64)) -> bool {
    let w = winding(rings, p);
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// The distance from `p` to the nearest edge of `rings`, mm (infinite without edges).
fn edge_distance(rings: &[Vec<(f64, f64)>], p: (f64, f64)) -> f64 {
    let mut best = f64::INFINITY;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { continue };
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            best = best.min((a.0 + dx * t - p.0).hypot(a.1 + dy * t - p.1));
        }
    }
    best
}

/// Whether the straight way from `a` to `b` stays hidden: each point of it, checked every quarter
/// of a millimetre, lies inside one of `regions` or within `edge_mm` of one's edge (travel along
/// a shared edge).
pub fn hidden(a: Pt, b: Pt, regions: &[Region], edge_mm: f64) -> bool {
    let (a, b) = (a.to_mm(), b.to_mm());
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / STEP_MM).ceil().clamp(1.0, MAX_CHECKS) as usize;
    (0..=n).all(|s| {
        let t = s as f64 / n as f64;
        let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        regions.iter().any(|(rings, rule)| inside(rings, *rule, p) || (edge_mm > 0.0 && edge_distance(rings, p) <= edge_mm))
    })
}

/// The stitches that carry the needle from `a` to `b` in a straight line, none longer than
/// `stitch_mm` (at least 0.5 mm): the points between, `b` itself not included.
pub fn stitched(a: Pt, b: Pt, stitch_mm: f64) -> Vec<Pt> {
    let gap = a.dist(b) / crate::UNITS_PER_MM;
    let n = (gap / stitch_mm.max(0.5)).ceil().clamp(1.0, 1_000.0) as i32;
    let (a, b) = (a.to_mm(), b.to_mm());
    (1..n)
        .filter_map(|s| {
            let t = f64::from(s) / f64::from(n);
            Pt::from_mm(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> Vec<Vec<(f64, f64)>> {
        vec![vec![(x, y), (x + side, y), (x + side, y + side), (x, y + side)]]
    }

    #[test]
    fn a_way_inside_or_along_an_edge_is_hidden() {
        let sq = square(0.0, 0.0, 10.0);
        let r: [Region; 1] = [(&sq, FillRule::NonZero)];
        let p = |x: f64, y: f64| Pt::from_mm(x, y).unwrap();
        assert!(hidden(p(1.0, 1.0), p(9.0, 9.0), &r, 0.0));
        assert!(!hidden(p(1.0, 1.0), p(15.0, 1.0), &r, 0.0));
        // Along the top edge, just outside: hidden only with an edge allowance.
        assert!(!hidden(p(1.0, -0.2), p(9.0, -0.2), &r, 0.0));
        assert!(hidden(p(1.0, -0.2), p(9.0, -0.2), &r, 0.3));
        // A hole, by the even-odd rule.
        let ring = vec![square(0.0, 0.0, 10.0).remove(0), square(3.0, 3.0, 4.0).remove(0)];
        assert!(!inside(&ring, FillRule::EvenOdd, (5.0, 5.0)));
        assert!(inside(&ring, FillRule::EvenOdd, (1.0, 5.0)));
    }

    #[test]
    fn stitched_travel_is_in_short_equal_stitches() {
        let (a, b) = (Pt { x: 0, y: 0 }, Pt { x: 100, y: 0 });
        assert_eq!(stitched(a, b, 2.5), vec![Pt { x: 25, y: 0 }, Pt { x: 50, y: 0 }, Pt { x: 75, y: 0 }]);
        assert!(stitched(a, Pt { x: 10, y: 0 }, 2.5).is_empty());
    }
}
