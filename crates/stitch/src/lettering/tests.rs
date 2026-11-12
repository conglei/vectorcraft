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

fn letter(rings: Vec<Vec<P>>) -> Letter {
    Letter { rings, rgb: [0, 0, 0], word: 0 }
}

/// The area of a polygon.
fn area(r: &[P]) -> f64 {
    let n = r.len();
    (0..n).map(|i| r[i].0 * r[(i + 1) % n].1 - r[(i + 1) % n].0 * r[i].1).sum::<f64>().abs() / 2.0
}

/// The area the columns cover, overlaps counted twice.
fn column_area(s: &LetterShape) -> f64 {
    s.columns
        .iter()
        .map(|c| {
            let mut poly = c.left.clone();
            poly.extend(c.right.iter().rev());
            area(&poly)
        })
        .sum()
}

fn shape(rings: Vec<Vec<P>>) -> LetterShape {
    shape_letter(&rings, LetterMode::Satin, &LetteringParams::default()).unwrap()
}

fn sew(rings: Vec<Vec<P>>, mode: LetterMode) -> SewnLetter {
    sew_letters(&[letter(rings)], mode, &LetteringParams::default(), MAX_STEPS).unwrap().remove(0)
}

#[test]
fn an_i_bar_is_one_column_as_wide_as_the_bar() {
    let s = shape(vec![rect(0.0, 0.0, 3.0, 15.0)]);
    assert_eq!(s.mode, LetterMode::Satin);
    assert_eq!(s.columns.len(), 1, "{s:?}");
    let c = &s.columns[0];
    let widths: Vec<f64> = c.left.iter().zip(&c.right).map(|(a, b)| dist(*a, *b)).collect();
    let mid = widths[widths.len() / 2];
    assert!((mid - 3.0).abs() < 0.15, "{mid}");
    // The column covers the bar end to end (the ends a little inside).
    let ys: Vec<f64> = c.left.iter().chain(&c.right).map(|p| p.1).collect();
    let (lo, hi) = ys.iter().fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(*y), hi.max(*y)));
    assert!(lo < 0.5 && hi > 14.5, "{lo}..{hi}");
    let cover = column_area(&s) / 45.0;
    assert!((0.9..1.1).contains(&cover), "{cover}");
}

#[test]
fn an_o_ring_is_a_closed_column_round_the_hole() {
    let s = shape(vec![circle(10.0, 10.0, 7.0, false), circle(10.0, 10.0, 4.0, true)]);
    assert_eq!(s.mode, LetterMode::Satin);
    assert!(!s.columns.is_empty());
    let ring = std::f64::consts::PI * (49.0 - 16.0);
    let cover = column_area(&s) / ring;
    assert!((0.85..1.2).contains(&cover), "{cover}");
    let sewn = sew(vec![circle(10.0, 10.0, 7.0, false), circle(10.0, 10.0, 4.0, true)], LetterMode::Satin);
    assert_eq!(sewn.runs.len(), 1, "no jump inside the O");
    for q in &sewn.runs[0] {
        let (x, y) = q.to_mm();
        let r = (x - 10.0).hypot(y - 10.0);
        assert!((3.5..=7.5).contains(&r), "{r}");
    }
}

#[test]
fn an_l_and_a_t_meet_without_a_gap_and_sew_in_one_go() {
    let l = vec![rect(0.0, 0.0, 3.0, 15.0), rect(0.0, 12.0, 10.0, 15.0)];
    let s = shape(l.clone());
    assert!(s.columns.len() >= 2, "{}", s.columns.len());
    let cover = column_area(&s) / (45.0 + 21.0);
    assert!((0.9..1.3).contains(&cover), "{cover}");
    let sewn = sew(l, LetterMode::Satin);
    assert_eq!(sewn.runs.len(), 1, "no jump or trim inside a letter");
    let t = vec![rect(0.0, 0.0, 12.0, 3.0), rect(4.5, 0.0, 7.5, 15.0)];
    let s = shape(t.clone());
    assert!(s.columns.len() >= 3, "{}", s.columns.len());
    let cover = column_area(&s) / (36.0 + 36.0);
    assert!((0.9..1.35).contains(&cover), "{cover}");
    assert_eq!(sew(t, LetterMode::Satin).runs.len(), 1);
}

#[test]
fn a_thin_bar_is_a_run_and_a_wide_blob_a_fill() {
    let s = shape(vec![rect(0.0, 0.0, 0.6, 12.0)]);
    assert!(s.columns.is_empty(), "{:?}", s.columns);
    assert_eq!(s.runs.len(), 1);
    let s = shape(vec![rect(0.0, 0.0, 20.0, 20.0)]);
    assert_eq!(s.mode, LetterMode::Fill);
    let sewn = sew(vec![rect(0.0, 0.0, 20.0, 20.0)], LetterMode::Satin);
    assert_eq!(sewn.mode, LetterMode::Fill);
    assert!(sewn.runs.iter().map(Vec::len).sum::<usize>() > 200);
}

#[test]
fn small_letters_are_running_stitch_along_the_skeleton() {
    let sewn = sew(vec![rect(0.0, 0.0, 0.5, 4.0), rect(0.0, 3.5, 2.5, 4.0)], LetterMode::Run);
    assert_eq!(sewn.mode, LetterMode::Run);
    assert_eq!(sewn.columns, 0);
    // No stitch longer than the run length (plus rounding).
    for r in &sewn.runs {
        for w in r.windows(2) {
            assert!(w[0].dist(w[1]) <= 16.0, "{:?}", w);
        }
    }
    assert_eq!(mode_for(4.0, &LetteringParams::default()), LetterMode::Run);
    assert_eq!(mode_for(9.0, &LetteringParams::default()), LetterMode::Satin);
}

#[test]
fn a_dot_and_a_stem_are_two_parts_joined_by_a_jump() {
    let i = vec![rect(0.0, 4.0, 2.0, 15.0), circle(1.0, 1.5, 1.2, false)];
    let sewn = sew(i, LetterMode::Satin);
    assert_eq!(sewn.runs.len(), 2, "{:?}", sewn.runs.len());
}

#[test]
fn hostile_outlines_are_refused_or_sewn_without_panicking() {
    assert!(shape_letter(&[vec![(f64::NAN, 0.0), (1.0, 1.0), (0.0, 1.0)]], LetterMode::Satin, &LetteringParams::default()).is_err());
    let many: Vec<P> = (0..30_000).map(|i| (f64::from(i).sin() * 5.0, f64::from(i).cos() * 5.0)).collect();
    assert!(shape_letter(&[many], LetterMode::Satin, &LetteringParams::default()).is_err());
    assert!(shape(vec![]).columns.is_empty());
    assert!(shape(vec![vec![(1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]]).columns.is_empty());
    // A huge letter still has a bounded grid.
    let s = shape(vec![rect(0.0, 0.0, 3000.0, 30.0)]);
    assert_eq!(s.mode, LetterMode::Fill);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Random outlines never panic; no column is narrower than the minimum and no stitch is
    /// longer than the maximum.
    #[test]
    fn random_outlines_keep_the_rules(
        pts in prop::collection::vec((0.0f64..20.0, 0.0f64..20.0), 3..24),
        min_column in 0.5f64..2.0, max in 2.0f64..12.1,
    ) {
        let p = LetteringParams { min_column, max_stitch: max, ..Default::default() };
        let s = shape_letter(std::slice::from_ref(&pts), LetterMode::Satin, &p).unwrap();
        for c in &s.columns {
            prop_assert!(c.min_width() >= min_column - 1e-9, "{} < {}", c.min_width(), min_column);
        }
        let sewn = sew_letters(&[letter(vec![pts])], LetterMode::Satin, &p, MAX_STEPS).unwrap();
        for r in sewn.iter().flat_map(|l| &l.runs) {
            for w in r.windows(2) {
                prop_assert!(w[0].dist(w[1]) / 10.0 <= max + 1e-9, "{:?} over {}", w, max);
            }
        }
    }
}
