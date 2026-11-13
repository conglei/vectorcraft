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
    assert_eq!(s.columns.len(), 1, "the L turns its corner as one column");
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
    let s = shape(vec![rect(0.0, 0.0, 0.4, 12.0)]);
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

/// The underlay's 2 mm stitches, rounded to plan units, passed a 2 mm longest stitch (found by
/// `random_outlines_keep_the_rules`): lettering's runs are held under the longest stitch.
#[test]
fn underlay_stitches_stay_under_a_short_longest_stitch() {
    let pts = vec![
        (0.0, 0.0),
        (0.0, 11.171373038572924),
        (14.225013028969077, 4.636847008723347),
        (19.66455935951981, 1.6280633049364854),
        (13.527165627949577, 15.26209117467225),
    ];
    let p = LetteringParams { min_column: 0.5, max_stitch: 2.0, ..Default::default() };
    let sewn = sew_letters(&[letter(vec![pts])], LetterMode::Satin, &p, MAX_STEPS).unwrap();
    assert_eq!(sewn[0].mode, LetterMode::Satin);
    for w in sewn.iter().flat_map(|l| &l.runs).flat_map(|r| r.windows(2)) {
        assert!(w[0].dist(w[1]) <= 20.0, "{w:?}");
    }
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
            prop_assert!(c.min_width() + 2.0 * p.compensation >= min_column - 1e-9, "{} < {}", c.min_width(), min_column);
        }
        let sewn = sew_letters(&[letter(vec![pts])], LetterMode::Satin, &p, MAX_STEPS).unwrap();
        for r in sewn.iter().flat_map(|l| &l.runs) {
            for w in r.windows(2) {
                prop_assert!(w[0].dist(w[1]) / 10.0 <= max + 1e-9, "{:?} over {}", w, max);
            }
        }
    }
}

/// Whether segments `ab` and `cd` cross, each strictly inside the other.
fn crosses(a: P, b: P, c: P, d: P) -> bool {
    let side = |p: P, q: P, r: P| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (d1, d2, d3, d4) = (side(c, d, a), side(c, d, b), side(a, b, c), side(a, b, d));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

/// The column's rails don't cross each other, and no two rungs cross.
fn assert_rails_never_cross(c: &Column) {
    let n = c.left.len();
    for i in 0..n - 1 {
        for j in 0..n - 1 {
            assert!(!crosses(c.left[i], c.left[i + 1], c.right[j], c.right[j + 1]), "the rails cross at rungs {i} and {j}");
            if j > i + 1 {
                assert!(
                    !crosses(c.left[i], c.right[i], c.left[j], c.right[j]),
                    "rungs {i} and {j} cross: {:?}",
                    (i.saturating_sub(2)..=j + 2).map(|k| (c.left[k], c.right[k])).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// The distance from `q` to the nearest edge of `rings`.
fn to_outline(rings: &[Vec<P>], q: P) -> f64 {
    let mut best = f64::INFINITY;
    for r in rings {
        for i in 0..r.len() {
            best = best.min(crate::fill::seg_dist(q, r[i], r[(i + 1) % r.len()]));
        }
    }
    best
}

/// Every point of the outline at least `margin` inside it lies between two rungs of a column.
fn assert_covered(rings: &[Vec<P>], columns: &[Column], margin: f64) {
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for q in rings.iter().flatten() {
        lo = (lo.0.min(q.0), lo.1.min(q.1));
        hi = (hi.0.max(q.0), hi.1.max(q.1));
    }
    let mut missed = vec![];
    let step = 0.2;
    // Off the round numbers, so no point lies on a rung shared by two quads.
    let mut y = lo.1 + 0.013;
    while y <= hi.1 {
        let mut x = lo.0 + 0.017;
        while x <= hi.0 {
            let q = (x, y);
            if inside_rings(rings, q) && to_outline(rings, q) >= margin {
                let held = columns
                    .iter()
                    .any(|c| (0..c.left.len() - 1).any(|i| inside_rings(&[vec![c.left[i], c.left[i + 1], c.right[i + 1], c.right[i]]], q)));
                if !held {
                    missed.push(q);
                }
            }
            x += step;
        }
        y += step;
    }
    assert!(missed.is_empty(), "{} points not covered, such as {:?}", missed.len(), &missed[..missed.len().min(6)]);
}

fn ring_15() -> Vec<Vec<P>> {
    let mut hole = rect(3.0, 3.0, 12.0, 12.0);
    hole.reverse();
    vec![rect(0.0, 0.0, 15.0, 15.0), hole]
}

/// A V: two 3 mm bars from the top to the bottom middle, 46° apart, cut square at the top and
/// flat at the bottom, as a letter's point is.
fn v_letter() -> Vec<Vec<P>> {
    let (a, b, t) = ((1.5, 0.0), (13.5, 0.0), (7.5, 14.0));
    let unit = |p: P, q: P| {
        let l = (q.0 - p.0).hypot(q.1 - p.1);
        ((q.0 - p.0) / l, (q.1 - p.1) / l)
    };
    let (dl, dr) = (unit(a, t), unit(b, t));
    // Outer normals: away from the middle.
    let nl = (-dl.1, dl.0);
    let nl = if nl.0 < 0.0 { nl } else { (-nl.0, -nl.1) };
    let nr = (-dr.1, dr.0);
    let nr = if nr.0 > 0.0 { nr } else { (-nr.0, -nr.1) };
    let at = |p: P, n: P, s: f64| (p.0 + n.0 * s, p.1 + n.1 * s);
    let (p1, p2, p3, p4) = (at(a, nl, 1.5), at(a, nl, -1.5), at(b, nr, -1.5), at(b, nr, 1.5));
    // Where the inner edges meet, and the outer edges cross y = 15.5.
    let meet = |p: P, d: P, q: P, e: P| {
        let den = d.0 * e.1 - d.1 * e.0;
        let s = ((q.0 - p.0) * e.1 - (q.1 - p.1) * e.0) / den;
        (p.0 + d.0 * s, p.1 + d.1 * s)
    };
    let inner = meet(p2, dl, p3, dr);
    let on_y = |p: P, d: P, y: f64| (p.0 + d.0 * (y - p.1) / d.1, y);
    vec![vec![p1, p2, inner, p3, p4, on_y(p4, dr, 15.5), on_y(p1, dl, 15.5)]]
}

/// STI-38: a stroke turning a sharp corner is one column whose rails mitre round it, not two
/// columns overlapping at the corner.
#[test]
fn an_l_a_v_and_a_square_ring_turn_their_corners_as_one_column() {
    let l = vec![rect(0.0, 0.0, 3.0, 15.0), rect(0.0, 12.0, 10.0, 15.0)];
    for (name, rings) in [("L", l), ("V", v_letter()), ("ring", ring_15())] {
        let s = shape(rings.clone());
        assert_eq!(s.columns.len(), 1, "{name}: {} columns", s.columns.len());
        assert_rails_never_cross(&s.columns[0]);
        assert_covered(&rings, &s.columns, 0.35);
        // Sewn in one go, its stitches inside the letter.
        let sewn = sew(rings.clone(), LetterMode::Satin);
        assert_eq!((sewn.runs.len(), sewn.columns), (1, 1), "{name}");
        for q in &sewn.runs[0] {
            let p = q.to_mm();
            assert!(inside_rings(&rings, p) || to_outline(&rings, p) < 0.45, "{name}: {p:?} outside");
        }
    }
}

/// A walk inside a letter's ink is hidden by the satin sewn over it; one that leaves the ink, or
/// crosses a hole, is not.
#[test]
fn a_walk_is_hidden_only_inside_the_ink() {
    // A 10 mm square with a 4 mm hole, as glyph outlines wind: outer counter-clockwise, hole
    // clockwise.
    let outer = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    let hole = vec![(3.0, 3.0), (3.0, 7.0), (7.0, 7.0), (7.0, 3.0)];
    let rings = vec![outer, hole];
    assert!(hidden((1.0, 1.0), (9.0, 1.0), &rings), "along the bottom bar");
    assert!(hidden((1.0, 1.0), (1.0, 9.0), &rings), "up the left bar");
    assert!(!hidden((1.0, 5.0), (9.0, 5.0), &rings), "across the hole");
    assert!(!hidden((1.0, 1.0), (15.0, 1.0), &rings), "out of the letter");
    assert!(!hidden((1.0, 1.0), (f64::NAN, 1.0), &rings), "not finite");
    assert!(hidden((2.0, 2.0), (2.0, 2.0), &rings), "no distance at all");
}
