use proptest::prelude::*;

use super::*;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<P> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// A circle, counter-clockwise or (`hole`) clockwise.
fn circle(cx: f64, cy: f64, r: f64, hole: bool) -> Vec<P> {
    let n = 96;
    let mut v: Vec<P> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    if hole {
        v.reverse();
    }
    v
}

fn auto(rings: Vec<Vec<P>>) -> AutoShape {
    auto_shape(&rings, &AutoParams::default()).unwrap()
}

/// The area of a polygon.
fn area(r: &[P]) -> f64 {
    let n = r.len();
    (0..n).map(|i| r[i].0 * r[(i + 1) % n].1 - r[(i + 1) % n].0 * r[i].1).sum::<f64>().abs() / 2.0
}

/// The area the fill outline covers, holes counted out.
fn fill_area(s: &AutoShape) -> f64 {
    let outer: f64 = s.fill.iter().map(|r| area(r)).fold(0.0, f64::max);
    2.0 * outer - s.fill.iter().map(|r| area(r)).sum::<f64>()
}

#[test]
fn a_1mm_line_is_a_run_and_a_4mm_bar_a_satin_column() {
    let s = auto(vec![rect(0.0, 0.0, 1.0, 20.0)]);
    assert_eq!(s.kind, AutoKind::Run, "{s:?}");
    assert_eq!((s.columns.len(), s.runs.len(), s.fill.len()), (0, 1, 0));
    assert!(length(&s.runs[0]) > 18.0, "{}", length(&s.runs[0]));
    let s = auto(vec![rect(0.0, 0.0, 4.0, 30.0)]);
    assert_eq!(s.kind, AutoKind::Satin);
    assert_eq!(s.columns.len(), 1, "{s:?}");
    let c = &s.columns[0];
    let mid = dist(c.left[c.left.len() / 2], c.right[c.right.len() / 2]);
    assert!((mid - 4.0).abs() < 0.15, "{mid}");
}

#[test]
fn a_20mm_disc_is_a_fill_and_a_small_disc_a_satin_dot() {
    let s = auto(vec![circle(10.0, 10.0, 10.0, false)]);
    assert_eq!(s.kind, AutoKind::Fill, "{:?}", s.kind);
    assert_eq!(s.fill.len(), 1);
    assert!(s.columns.is_empty() && s.runs.is_empty());
    // A compact blob narrower than satin's widest but over half of it is still a fill.
    assert_eq!(auto(vec![circle(0.0, 0.0, 3.0, false)]).kind, AutoKind::Fill);
    let dot = auto(vec![circle(0.0, 0.0, 1.5, false)]);
    assert_eq!(dot.kind, AutoKind::Satin, "{dot:?}");
}

#[test]
fn an_l_is_columns_without_fill() {
    let s = auto(vec![rect(0.0, 0.0, 4.0, 25.0), rect(0.0, 21.0, 20.0, 25.0)]);
    assert_eq!(s.kind, AutoKind::Satin, "{:?}", s.kind);
    assert!(!s.columns.is_empty() && s.fill.is_empty());
    // A T: columns only; the stem's stretch inside the bar is the bar's, not a run.
    let t = auto(vec![vec![(0.0, 0.0), (20.0, 0.0), (20.0, 4.0), (12.0, 4.0), (12.0, 26.0), (8.0, 26.0), (8.0, 4.0), (0.0, 4.0)]]);
    assert_eq!(t.kind, AutoKind::Satin);
    assert!(t.runs.is_empty(), "{:?}", t.runs);
}

#[test]
fn a_ring_is_a_closed_column_and_a_wide_ring_a_fill_with_its_hole() {
    let s = auto(vec![circle(10.0, 10.0, 10.0, false), circle(10.0, 10.0, 7.0, true)]);
    assert_eq!(s.kind, AutoKind::Satin, "{:?}", s.kind);
    assert_eq!(s.columns.len(), 1);
    let c = &s.columns[0];
    let widest = c.left.iter().zip(&c.right).map(|(a, b)| dist(*a, *b)).fold(0.0, f64::max);
    assert!(widest < 3.5, "every rung crosses the ring, not its hole: {widest}");
    let s = auto(vec![circle(20.0, 20.0, 20.0, false), circle(20.0, 20.0, 5.0, true)]);
    assert_eq!(s.kind, AutoKind::Fill);
    assert_eq!(s.fill.len(), 2, "the hole stays");
}

#[test]
fn a_wide_blob_with_a_thin_tail_is_a_fill_and_a_satin_tail() {
    // A 20 mm disc with a 4 mm bar 20 mm long off its right side.
    let rings = vec![circle(10.0, 10.0, 10.0, false), rect(18.0, 8.0, 40.0, 12.0)];
    let s = auto(rings);
    assert_eq!(s.kind, AutoKind::Mixed, "{:?}", s.kind);
    assert_eq!(s.columns.len(), 1, "{:?}", s.columns.len());
    assert!(!s.fill.is_empty());
    // The fill is about the disc; the column is the tail, reaching a little into the fill.
    let a = fill_area(&s);
    let disc = std::f64::consts::PI * 100.0;
    assert!((0.9 * disc..1.1 * disc).contains(&a), "{a} vs {disc}");
    let xs: Vec<f64> = s.columns[0].left.iter().map(|p| p.0).collect();
    let (lo, hi) = xs.iter().fold((f64::MAX, f64::MIN), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
    assert!(lo < 20.5 && lo > 17.0 && hi > 39.0, "{lo}..{hi}");
    let fill_right = s.fill.iter().flatten().map(|p| p.0).fold(f64::MIN, f64::max);
    assert!(fill_right > lo + 0.1 && fill_right < lo + 1.0, "the column overlaps the fill: {lo} < {fill_right}");
}

#[test]
fn a_wide_square_is_one_fill_with_its_own_corners() {
    let s = auto(vec![rect(0.0, 0.0, 30.0, 20.0)]);
    assert_eq!(s.kind, AutoKind::Fill, "{s:?}");
    assert_eq!(s.fill, vec![rect(0.0, 0.0, 30.0, 20.0)], "the outline as drawn");
}

#[test]
fn strokes_are_classed_by_weight() {
    let p = AutoParams::default();
    assert_eq!(stroke_kind(0.5, &p), StrokeKind::Run);
    assert_eq!(stroke_kind(1.0, &p), StrokeKind::Run);
    assert_eq!(stroke_kind(3.0, &p), StrokeKind::Satin);
    assert_eq!(stroke_kind(12.0, &p), StrokeKind::Fill);
    assert_eq!(stroke_kind(f64::NAN, &p), StrokeKind::Run);
}

#[test]
fn the_mask_tracer_keeps_holes() {
    // A 4 × 4 square with a 2 × 2 hole.
    let mut m = vec![true; 16];
    for i in [5, 6, 9, 10] {
        m[i] = false;
    }
    let loops = trace_mask(&m, 4, 4);
    assert_eq!(loops.len(), 2);
    let simple: Vec<_> = loops.iter().map(|l| simplify(l, 0.1)).collect();
    assert!(simple.iter().all(|l| l.len() >= 4 && l.len() <= 8), "{simple:?}");
}

#[test]
fn hostile_outlines_are_refused_or_classed_without_panicking() {
    assert!(auto_shape(&[vec![(f64::NAN, 0.0), (1.0, 1.0), (0.0, 1.0)]], &AutoParams::default()).is_err());
    assert_eq!(auto(vec![]).kind, AutoKind::None);
    assert_eq!(auto(vec![vec![(1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]]).kind, AutoKind::None);
    assert_eq!(auto(vec![rect(0.0, 0.0, 3000.0, 30.0)]).kind, AutoKind::Fill);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Random outlines never panic, and no column is narrower than the minimum as drawn.
    #[test]
    fn random_outlines_keep_the_minimum_column(
        pts in prop::collection::vec((0.0f64..30.0, 0.0f64..30.0), 3..24),
        min_column in 0.5f64..2.0, max_satin in 3.0f64..12.0,
    ) {
        let p = AutoParams { min_column, max_satin };
        let s = auto_shape(std::slice::from_ref(&pts), &p).unwrap();
        for c in &s.columns {
            prop_assert!(c.min_width() >= min_column, "{} < {}", c.min_width(), min_column);
        }
        for r in &s.fill {
            prop_assert!(r.iter().all(|q| q.0.is_finite() && q.1.is_finite()));
        }
    }
}
