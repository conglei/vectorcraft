//! Overlap removal and routed travel, through `from_document`.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId, StitchProps};
use vectorcraft_geom::{PathData, Point, SubPath};

use crate::{FromDocOptions, PT_PER_MM, Pt, Step, from_document, stats};

const RED: [u8; 3] = [200, 30, 30];
const BLUE: [u8; 3] = [30, 30, 200];

fn mm(x: f64, y: f64) -> Point {
    Point::new(x * PT_PER_MM, y * PT_PER_MM)
}

/// A filled square, `side` mm, at (`x`, `y`) mm, with no stroke.
fn square(id: u64, x: f64, y: f64, side: f64, rgb: [u8; 3]) -> Node {
    let pts = [mm(x, y), mm(x + side, y), mm(x + side, y + side), mm(x, y + side)];
    Node::path(
        NodeId(id),
        PathData::single(SubPath::polyline(&pts, true)),
        Appearance::basic(Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])), Paint::None, 0.0),
    )
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

fn with_props(mut n: Node, f: impl FnOnce(&mut StitchProps)) -> Node {
    let mut p = n.stitch_props().copied().unwrap_or_default();
    f(&mut p);
    n.set_stitch_props(p);
    n
}

/// The needle points of the first block (the earlier square's thread).
fn first_block(d: &Document) -> Vec<Pt> {
    let plan = from_document(d, &FromDocOptions::default()).unwrap().plan;
    plan.blocks[0].steps.iter().filter_map(|s| if let Step::Stitch(p) = s { Some(*p) } else { None }).collect()
}

#[test]
fn a_later_fill_cuts_the_earlier_one_but_for_the_allowance() {
    // Two 20 mm squares, the blue one over the red's lower right quarter.
    let art =
        |overlap: Option<f64>| doc_of(vec![with_props(square(1, 0.0, 0.0, 20.0, RED), |p| p.overlap = overlap), square(2, 10.0, 10.0, 20.0, BLUE)]);
    let whole = first_block(&art(None));
    let cut = first_block(&art(Some(0.7)));
    assert!(cut.len() < whole.len() * 9 / 10, "{} → {}", cut.len(), whole.len());
    // No red stitch lies deeper than the allowance (and a stitch's reach) inside the blue square.
    let deep = |p: &Pt| p.x > 100 + 7 + 3 && p.y > 100 + 7 + 3;
    assert!(!cut.iter().any(deep), "{:?}", cut.iter().find(|p| deep(p)));
    assert!(whole.iter().any(deep));
    // The covered area keeps red under the blue edge: within the allowance band.
    let band = |p: &Pt| (p.x > 100 && p.y > 100) && (p.x <= 100 + 10 || p.y <= 100 + 10);
    assert!(cut.iter().any(band));
    // Outside the overlap the red fill still reaches its corners.
    assert!(cut.iter().any(|p| p.x < 10 && p.y < 10) && cut.iter().any(|p| p.x > 190 && p.y < 50));
    // The later square is not cut.
    let plan = |d: &Document| from_document(d, &FromDocOptions::default()).unwrap().plan;
    let blue = |d: &Document| plan(d).blocks[1].steps.len();
    assert_eq!(blue(&art(None)), blue(&art(Some(0.7))));
    // Nothing later: nothing cut.
    let alone = doc_of(vec![square(2, 10.0, 10.0, 20.0, BLUE), with_props(square(1, 0.0, 0.0, 20.0, RED), |p| p.overlap = Some(0.7))]);
    let mut d = alone.clone();
    if let Some(n) = Arc::make_mut(&mut d.layers[0]).children_mut() {
        let last = Arc::make_mut(&mut n[1]);
        last.set_stitch_props(StitchProps::default());
    }
    assert_eq!(stats(&plan(&alone)).stitches, stats(&plan(&d)).stitches);
}

#[test]
fn routed_travel_replaces_a_trim_under_a_later_fill() {
    // Two red squares 2 mm apart, then a red one over both, sewn last: wherever the first ends
    // and the second starts, the way between is under it.
    let art = |route: Option<f64>| {
        doc_of(vec![
            square(1, 0.0, 0.0, 10.0, RED),
            with_props(square(2, 12.0, 0.0, 10.0, RED), |p| p.route = route),
            square(3, -1.0, -1.0, 24.0, RED),
        ])
    };
    let trims = |d: &Document| stats(&from_document(d, &FromDocOptions::default()).unwrap().plan).trims;
    let before = trims(&art(None));
    let routed = trims(&art(Some(20.0)));
    assert!(before >= 1, "{before}");
    assert!(routed < before, "{before} → {routed}");
    // Too short a limit keeps the trim.
    assert_eq!(trims(&art(Some(0.1))), before);
    // Without the covering fill, the way is open: the trim stays.
    let open = doc_of(vec![square(1, 0.0, 0.0, 10.0, RED), with_props(square(2, 30.0, 0.0, 10.0, RED), |p| p.route = Some(20.0))]);
    let shut = doc_of(vec![square(1, 0.0, 0.0, 10.0, RED), square(2, 30.0, 0.0, 10.0, RED)]);
    assert_eq!(trims(&open), trims(&shut));
}
