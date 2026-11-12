//! Stitch properties on objects ([`vectorcraft_doc::StitchProps`]) in [`crate::from_document`].

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId, StitchProps, StitchType};
use vectorcraft_geom::{Affine, PathData, Point, SubPath};

use crate::{FillOptions, FillType, FromDocOptions, PT_PER_MM, Step, StitchPlan, from_document, stats};

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn mm(x: f64, y: f64) -> Point {
    Point::new(x * PT_PER_MM, y * PT_PER_MM)
}

fn square_mm(side: f64) -> PathData {
    PathData::single(SubPath::polyline(&[mm(0.0, 0.0), mm(side, 0.0), mm(side, side), mm(0.0, side)], true))
}

fn paint(rgb: [u8; 3]) -> Paint {
    Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2]))
}

/// A path painted with `fill` and `stroke` (each optional), the stroke 1 pt.
fn path(id: u64, data: PathData, fill: Option<[u8; 3]>, stroke: Option<[u8; 3]>, props: StitchProps) -> Node {
    let mut n = Node::path(NodeId(id), data, Appearance::basic(fill.map_or(Paint::None, paint), stroke.map_or(Paint::None, paint), 1.0));
    n.set_stitch_props(props);
    n
}

/// A document whose one layer holds `art`.
fn doc_of(art: Vec<Node>) -> Document {
    let mut doc = Document::new(500.0, 500.0);
    let mut layer = Node::layer(NodeId(1000), "Art", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = art.into_iter().map(Arc::new).collect();
    }
    doc.layers = vec![Arc::new(layer)];
    doc
}

fn plan(doc: &Document) -> StitchPlan {
    from_document(doc, &FromDocOptions::default()).unwrap().plan
}

fn kind(t: StitchType) -> StitchProps {
    StitchProps { kind: t, ..Default::default() }
}

fn colours(p: &StitchPlan) -> Vec<[u8; 3]> {
    p.blocks.iter().map(|b| b.thread.rgb).collect()
}

/// Stitches are made from the outline at export time: a filled square scaled by 2 has about 4
/// times the stitches, at the same row spacing (the density), with and without properties.
#[test]
fn a_scaled_object_is_sewn_again_at_the_same_density() {
    for props in [StitchProps::default(), StitchProps { kind: StitchType::Tatami, fill_spacing: Some(0.6), ..Default::default() }] {
        let small = path(1, square_mm(20.0), Some(BLUE), None, props);
        let mut big = small.clone();
        if let Some(p) = big.path_data_mut() {
            *p = p.transformed(Affine::scale(2.0));
        }
        let (pa, pb) = (plan(&doc_of(vec![small])), plan(&doc_of(vec![big])));
        let (a, b) = (stats(&pa), stats(&pb));
        let ratio = b.stitches as f64 / a.stitches as f64;
        assert!((3.5..=4.5).contains(&ratio), "{} → {} stitches: × {ratio}", a.stitches, b.stitches);
        assert!((b.width_mm / a.width_mm - 2.0).abs() < 0.05);
        // Twice the size each way: the rows stay as far apart, so there are twice as many.
        let rows = |p: &StitchPlan| {
            let mut ys: Vec<i32> = p.points().map(|q| q.y).collect();
            ys.sort_unstable();
            ys.dedup();
            ys.len() as f64
        };
        let (ra, rb) = (rows(&pa), rows(&pb));
        assert!((rb / ra - 2.0).abs() < 0.2, "rows {ra} → {rb}");
    }
}

#[test]
fn an_object_set_not_sewn_is_left_out() {
    let d = doc_of(vec![
        path(1, square_mm(10.0), Some(BLUE), Some(RED), kind(StitchType::None)),
        path(2, square_mm(10.0), None, Some(RED), StitchProps::default()),
    ]);
    let out = from_document(&d, &FromDocOptions::default()).unwrap();
    assert_eq!(colours(&out.plan), vec![RED]);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    // Nothing at all to sew is the usual error.
    assert!(
        from_document(&doc_of(vec![path(1, square_mm(10.0), Some(BLUE), Some(RED), kind(StitchType::None))]), &FromDocOptions::default()).is_err()
    );
}

#[test]
fn one_stitch_type_sews_the_object_once() {
    let auto = plan(&doc_of(vec![path(1, square_mm(10.0), Some(BLUE), Some(RED), StitchProps::default())]));
    assert_eq!(colours(&auto), vec![BLUE, RED], "automatic: the fill, then the outline");
    // Running stitch: the outline only, in the stroke colour.
    let run = plan(&doc_of(vec![path(1, square_mm(10.0), Some(BLUE), Some(RED), kind(StitchType::Run))]));
    assert_eq!(colours(&run), vec![RED]);
    assert_eq!(run.points().count(), 17, "4 sides × 4 steps of 2.5 mm, back to the start");
    // Tatami: the region only, in the fill colour; without a fill, in the stroke's.
    let fill = plan(&doc_of(vec![path(1, square_mm(10.0), Some(BLUE), Some(RED), kind(StitchType::Tatami))]));
    assert_eq!(colours(&fill), vec![BLUE]);
    let outline_only = plan(&doc_of(vec![path(1, square_mm(10.0), None, Some(RED), kind(StitchType::Tatami))]));
    assert_eq!(colours(&outline_only), vec![RED]);
    assert_eq!(stats(&outline_only).stitches, stats(&fill).stitches, "the same rows, in the stroke colour");
    // Satin along a filled path without a stroke: in the fill colour, at the default width.
    let satin = plan(&doc_of(vec![path(1, square_mm(20.0), Some(BLUE), None, kind(StitchType::Satin))]));
    assert_eq!(colours(&satin), vec![BLUE]);
    let s = stats(&satin);
    assert!((s.width_mm - (20.0 + crate::DEFAULT_SATIN_WIDTH_MM)).abs() < 0.3, "{}", s.width_mm);
    // Manual: one needle point per anchor.
    let manual = plan(&doc_of(vec![path(1, square_mm(10.0), None, Some(RED), kind(StitchType::Manual))]));
    assert_eq!(manual.points().count(), 5);
}

#[test]
fn an_objects_numbers_override_the_documents() {
    let base = plan(&doc_of(vec![path(1, square_mm(20.0), Some(BLUE), None, StitchProps::default())]));
    let wide = StitchProps { fill_spacing: Some(0.8), ..Default::default() };
    let sparse = plan(&doc_of(vec![path(1, square_mm(20.0), Some(BLUE), None, wide)]));
    let ratio = stats(&base).stitches as f64 / stats(&sparse).stitches as f64;
    assert!((1.7..=2.3).contains(&ratio), "rows twice as far apart: × {ratio}");
    // A running stitch length of 5 mm: 2 steps a side.
    let long = StitchProps { kind: StitchType::Run, run_length: Some(5.0), ..Default::default() };
    assert_eq!(plan(&doc_of(vec![path(1, square_mm(10.0), None, Some(RED), long)])).points().count(), 9);
    // Values out of range (from a file) are ignored: the document's apply.
    let bad = StitchProps { kind: StitchType::Run, run_length: Some(99.0), ..Default::default() };
    assert_eq!(plan(&doc_of(vec![path(1, square_mm(10.0), None, Some(RED), bad)])).points().count(), 17);
    // An object's numbers win over the document's options, which still apply where it sets none.
    let o = FromDocOptions {
        fill: FillOptions { kind: FillType::Tatami, params: crate::FillParams { spacing: 0.8, ..Default::default() } },
        ..Default::default()
    };
    let doc_wide = from_document(&doc_of(vec![path(1, square_mm(20.0), Some(BLUE), None, StitchProps::default())]), &o).unwrap().plan;
    assert_eq!(stats(&doc_wide).stitches, stats(&sparse).stitches);
    let own = StitchProps { fill_spacing: Some(0.4), ..Default::default() };
    let back = from_document(&doc_of(vec![path(1, square_mm(20.0), Some(BLUE), None, own)]), &o).unwrap().plan;
    assert_eq!(stats(&back).stitches, stats(&base).stitches);
}

#[test]
fn groups_pass_their_properties_down_field_by_field() {
    let group_props = StitchProps { kind: StitchType::Run, run_length: Some(5.0), ..Default::default() };
    let a = path(1, square_mm(10.0), Some(BLUE), Some(RED), StitchProps::default());
    let b = path(2, square_mm(10.0), Some(BLUE), Some(RED), StitchProps { run_length: Some(2.5), ..Default::default() });
    let mut g = Node::group(NodeId(3), vec![Arc::new(a), Arc::new(b)]);
    g.set_stitch_props(group_props);
    let p = plan(&doc_of(vec![g]));
    assert_eq!(colours(&p), vec![RED], "both run: no fill");
    let stitched = p.blocks[0].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count();
    // The second square starts where the first ended, so its first stitch would sew nothing.
    assert_eq!(stitched, 9 + 16, "the first at the group's 5 mm, the second at its own 2.5 mm");
}

#[test]
fn an_opened_stitch_files_layer_ignores_stitch_properties() {
    let d = doc_of(vec![path(1, square_mm(10.0), None, Some(RED), StitchProps::default())]);
    let opened = crate::to_document(&plan(&d));
    let mut marked = opened.clone();
    let mut layer = (*marked.layers[0]).clone();
    layer.set_stitch_props(kind(StitchType::Satin));
    marked.layers = vec![Arc::new(layer)];
    let a = from_document(&opened, &FromDocOptions::default()).unwrap();
    let b = from_document(&marked, &FromDocOptions::default()).unwrap();
    assert_eq!(a.plan, b.plan);
    assert_eq!(b.marked_runs, 1);
}
