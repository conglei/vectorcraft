//! The sewing order and entry and exit points, through `from_document`.

use std::sync::Arc;

use proptest::prelude::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId, StitchProps};
use vectorcraft_geom::{PathData, Point, SubPath};

use crate::sequence::{Item, closest_join, cost};
use crate::{FromDocOptions, PT_PER_MM, Pt, Step, from_document, outline_fraction, outline_point, sequence, stats};

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn mm(x: f64, y: f64) -> Point {
    Point::new(x * PT_PER_MM, y * PT_PER_MM)
}

fn stroked(id: u64, pts: &[(f64, f64)], closed: bool, rgb: [u8; 3]) -> Node {
    let path = PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| mm(x, y)).collect::<Vec<_>>(), closed));
    Node::path(NodeId(id), path, Appearance::basic(Paint::None, Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])), 1.0))
}

fn doc_of(art: Vec<Node>) -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    let mut layer = Node::layer(NodeId(10_000), "Art", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = art.into_iter().map(Arc::new).collect();
    }
    d.layers = vec![Arc::new(layer)];
    d
}

fn first_stitch(d: &Document) -> Pt {
    let plan = from_document(d, &FromDocOptions::default()).unwrap().plan;
    plan.points().next().unwrap()
}

#[test]
fn the_design_order_is_sewn_and_unlisted_objects_follow() {
    let mut d = doc_of(vec![
        stroked(1, &[(0.0, 0.0), (10.0, 0.0)], false, RED),
        stroked(2, &[(0.0, 20.0), (10.0, 20.0)], false, BLUE),
        stroked(3, &[(0.0, 40.0), (10.0, 40.0)], false, RED),
    ]);
    let s = sequence(&d, &FromDocOptions::default()).unwrap();
    assert_eq!(s.items.iter().map(|i| i.id.0).collect::<Vec<_>>(), vec![1, 2, 3]);
    d.stitch.order = vec![NodeId(3), NodeId(77), NodeId(3)];
    let s = sequence(&d, &FromDocOptions::default()).unwrap();
    assert_eq!(s.items.iter().map(|i| i.id.0).collect::<Vec<_>>(), vec![3, 1, 2]);
    let plan = from_document(&d, &FromDocOptions::default()).unwrap().plan;
    // Red (3, then 1), then blue: one colour change fewer than document order.
    assert_eq!(plan.blocks.len(), 2);
    assert_eq!(plan.points().next(), Some(Pt { x: 0, y: 400 }));
}

#[test]
fn an_entry_point_turns_a_line_and_starts_a_closed_outline_there() {
    let mut line = stroked(1, &[(0.0, 0.0), (10.0, 0.0)], false, RED);
    line.set_stitch_props(StitchProps { entry: Some(1.0), ..Default::default() });
    assert_eq!(first_stitch(&doc_of(vec![line])), Pt { x: 100, y: 0 });
    let mut square = stroked(1, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true, RED);
    // Halfway round the outline: the opposite corner.
    square.set_stitch_props(StitchProps { entry: Some(0.5), ..Default::default() });
    let d = doc_of(vec![square]);
    assert_eq!(first_stitch(&d), Pt { x: 100, y: 100 });
    let plan = from_document(&d, &FromDocOptions::default()).unwrap().plan;
    let pts: Vec<Pt> = plan.points().collect();
    assert_eq!((pts.len(), pts.last()), (17, Some(&Pt { x: 100, y: 100 })), "the whole outline, closed where it started");
}

#[test]
fn an_exit_point_alone_turns_a_line_to_leave_there() {
    let mut line = stroked(1, &[(0.0, 0.0), (10.0, 0.0)], false, RED);
    line.set_stitch_props(StitchProps { exit: Some(0.0), ..Default::default() });
    let plan = from_document(&doc_of(vec![line]), &FromDocOptions::default()).unwrap().plan;
    assert_eq!(plan.points().last(), Some(Pt { x: 0, y: 0 }));
}

#[test]
fn the_design_start_and_end_turn_the_first_and_last_objects() {
    let mut d = doc_of(vec![stroked(1, &[(0.0, 0.0), (10.0, 0.0)], false, RED), stroked(2, &[(0.0, 20.0), (10.0, 20.0)], false, RED)]);
    d.stitch.start = Some(mm(12.0, 0.0));
    d.stitch.end = Some(mm(12.0, 20.0));
    let plan = from_document(&d, &FromDocOptions::default()).unwrap().plan;
    let pts: Vec<Pt> = plan.points().collect();
    assert_eq!(pts.first(), Some(&Pt { x: 100, y: 0 }));
    assert_eq!(pts.last(), Some(&Pt { x: 100, y: 200 }));
}

#[test]
fn outline_fractions_and_points_round_trip() {
    let square = stroked(1, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true, RED);
    assert_eq!(outline_point(&square, 0.25), Some(Pt { x: 100, y: 0 }));
    assert_eq!(outline_point(&square, 1.0), Some(Pt { x: 0, y: 0 }));
    assert_eq!(outline_point(&square, 1.5), None);
    let t = outline_fraction(&square, Pt { x: 103, y: 50 }).unwrap();
    assert!((t - 0.375).abs() < 1e-9, "{t}");
    for k in 0..=20 {
        let t = f64::from(k) / 20.0;
        let p = outline_point(&square, t).unwrap();
        let back = outline_point(&square, outline_fraction(&square, p).unwrap()).unwrap();
        assert!(p.dist(back) <= 1.0, "{t}: {p:?} {back:?}");
    }
}

/// The trims and travel (in units) `doc`'s plan has.
fn trims_and_travel(d: &Document) -> (usize, f64) {
    let plan = from_document(d, &FromDocOptions::default()).unwrap().plan;
    let st = stats(&plan);
    (st.trims, st.travel_mm * 10.0)
}

/// Persist the ways closest join picks as each object's preferred entry and exit.
fn join_closest(d: &mut Document) {
    let s = sequence(d, &FromDocOptions::default()).unwrap();
    let items: Vec<Item> = s.items.iter().map(|i| i.item.clone()).collect();
    let current: Vec<usize> = s.items.iter().map(|i| i.way).collect();
    let ways = closest_join(&items, &current, &s.ends);
    assert!(cost(&items, &ways, &s.ends).travel <= cost(&items, &current, &s.ends).travel + 1e-6);
    for (it, w) in s.items.iter().zip(ways) {
        let Some(way) = it.item.ways.get(w).copied() else { continue };
        let n = d.node_mut(it.id).unwrap();
        let entry = outline_fraction(n, way.entry);
        let exit = outline_fraction(n, way.exit);
        let mut p = n.stitch_props().copied().unwrap_or_default();
        p.join_entry = entry;
        p.join_exit = exit;
        n.set_stitch_props(p);
    }
}

fn stitches(d: &Document) -> usize {
    let plan = from_document(d, &FromDocOptions::default()).unwrap().plan;
    plan.blocks.iter().flat_map(|b| &b.steps).filter(|s| matches!(s, Step::Stitch(_))).count()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn every_object_is_sewn_once_in_any_order(
        lines in prop::collection::vec((0.0f64..100.0, 0.0f64..100.0, 0.0f64..100.0, 0.0f64..100.0, 0usize..2, prop::bool::ANY), 1..10),
        listed in prop::collection::vec(0u64..14, 0..14),
    ) {
        let art: Vec<Node> = lines.iter().enumerate().map(|(i, &(x0, y0, x1, y1, c, closed))| {
            let pts = if closed { vec![(x0, y0), (x1, y0 + 1.0), (x1 + 2.0, y1 + 3.0)] } else { vec![(x0, y0), (x1 + 1.0, y1)] };
            stroked(i as u64 + 1, &pts, closed, [RED, BLUE][c])
        }).collect();
        let mut d = doc_of(art);
        let n_before = stitches(&d);
        d.stitch.order = listed.into_iter().map(NodeId).collect();
        let s = sequence(&d, &FromDocOptions::default()).unwrap();
        let mut ids: Vec<u64> = s.items.iter().map(|i| i.id.0).collect();
        ids.sort_unstable();
        let mut natural: Vec<u64> = s.natural.iter().map(|i| i.0).collect();
        natural.sort_unstable();
        prop_assert_eq!(ids, natural);
        let n_after = stitches(&d);
        // Reordering only moves where runs join: a run joined to the last one may drop its first
        // stitch (or keep it), never more.
        prop_assert!(n_after.abs_diff(n_before) <= lines.len(), "{} → {}", n_before, n_after);
    }

    #[test]
    fn closest_join_never_adds_trims_or_travel_to_the_plan(
        lines in prop::collection::vec((0.0f64..100.0, 0.0f64..100.0, 0.0f64..100.0, 0.0f64..100.0, prop::bool::ANY), 1..10),
    ) {
        let art: Vec<Node> = lines.iter().enumerate().map(|(i, &(x0, y0, x1, y1, closed))| {
            let pts = if closed { vec![(x0, y0), (x0 + 8.0, y0), (x0 + 8.0, y0 + 8.0)] } else { vec![(x0, y0), (x1 + 1.0, y1)] };
            stroked(i as u64 + 1, &pts, closed, RED)
        }).collect();
        let mut d = doc_of(art);
        let (t0, v0) = trims_and_travel(&d);
        join_closest(&mut d);
        let (t1, v1) = trims_and_travel(&d);
        prop_assert!(t1 <= t0, "trims {} → {}", t0, t1);
        // Points stored as fractions come back within a unit of where they were chosen.
        prop_assert!(v1 <= v0 + 2.0 * lines.len() as f64, "travel {} → {}", v0, v1);
    }
}
