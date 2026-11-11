//! A document from a stitch plan: one stroked open path per run of stitches.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, Unit};
use vectorcraft_geom::{PathData, Point, Rect, SubPath};

use crate::{PT_PER_MM, Pt, Step, StitchPlan};

/// The width a thread is drawn at, in millimetres.
pub const THREAD_WIDTH_MM: f64 = 0.4;

/// Room left around the stitches on the artboard, in millimetres.
const MARGIN_MM: f64 = 10.0;
/// The object data key that marks the layer [`to_document`] makes.
const MARK: (&str, &str) = ("stitchFile", "1");

/// Whether `doc` was made from a stitch plan by [`to_document`] (a layer carries its mark), so
/// its paths are needle points to export unchanged.
pub fn is_stitch_document(doc: &Document) -> bool {
    doc.layers.iter().any(|l| l.extras().attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, v)| k == MARK.0 && v == MARK.1)))
}

fn point(p: Pt) -> Point {
    let (x, y) = p.to_mm();
    Point::new(x * PT_PER_MM, y * PT_PER_MM)
}

/// `plan` as a document in millimetres: one layer, `Stitches`, holding a group per block
/// (`Colour 1`, `Colour 2`, …) of open paths, one per run of stitches between trims, jumps and
/// stops, whose corner anchors are the needle points, stroked in the thread colour. Jumps are not
/// drawn. The one artboard is the stitches' extents grown by 10 mm on each side. The layer carries
/// the object data `stitchFile=1`, which [`is_stitch_document`] looks for.
pub fn to_document(plan: &StitchPlan) -> Document {
    let mut doc = Document::new(1.0, 1.0);
    doc.units = Unit::Millimeters;
    let mut extents: Option<Rect> = None;
    let mut groups: Vec<Arc<Node>> = Vec::with_capacity(plan.blocks.len());
    for (i, block) in plan.blocks.iter().enumerate() {
        let [r, g, b] = block.thread.rgb;
        let appearance = Appearance::basic(Paint::None, Paint::solid(Color::rgb8(r, g, b)), THREAD_WIDTH_MM * PT_PER_MM);
        let mut paths: Vec<Arc<Node>> = vec![];
        let mut run: Vec<Point> = vec![];
        let end_run = |run: &mut Vec<Point>, doc: &mut Document, paths: &mut Vec<Arc<Node>>| {
            if !run.is_empty() {
                let path = PathData::single(SubPath::polyline(run, false));
                paths.push(Arc::new(Node::path(doc.alloc_id(), path, appearance.clone())));
                run.clear();
            }
        };
        for step in &block.steps {
            match step {
                Step::Stitch(p) => {
                    let q = point(*p);
                    let r = Rect::from_points(q, q);
                    extents = Some(extents.map_or(r, |e| e.union(r)));
                    run.push(q);
                }
                Step::Jump(_) | Step::Trim | Step::Stop => end_run(&mut run, &mut doc, &mut paths),
            }
        }
        end_run(&mut run, &mut doc, &mut paths);
        let mut group = Node::group(doc.alloc_id(), paths);
        group.name = Some(format!("Colour {}", i + 1));
        groups.push(Arc::new(group));
    }
    let mut layer = Node::layer(doc.alloc_id(), "Stitches", LayerColor::Preset(0));
    layer.edit_attrs(|a| a.data.push((MARK.0.to_string(), MARK.1.to_string())));
    if let Some(children) = layer.children_mut() {
        *children = groups;
    }
    doc.layers = vec![Arc::new(layer)];
    let margin = MARGIN_MM * PT_PER_MM;
    let board = extents.unwrap_or_default().inflate(margin, margin);
    if let Some(a) = doc.artboards.first_mut() {
        a.rect = board;
    }
    doc
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::{NodeKind, Unit};

    use super::*;
    use crate::{Block, FromDocOptions, Mode, Pt, Step, StitchPlan, Thread, from_document};

    fn sample() -> StitchPlan {
        let p = |x: i32, y: i32| Pt { x, y };
        StitchPlan {
            name: String::new(),
            blocks: vec![
                Block {
                    thread: Thread { rgb: [200, 10, 20], name: None, code: None },
                    steps: vec![
                        Step::Stitch(p(0, 0)),
                        Step::Stitch(p(25, 0)),
                        Step::Stitch(p(50, 10)),
                        Step::Trim,
                        Step::Jump(p(300, 300)),
                        Step::Stitch(p(300, 300)),
                        Step::Stitch(p(320, 315)),
                    ],
                },
                Block {
                    thread: Thread { rgb: [0, 90, 255], name: None, code: None },
                    steps: vec![Step::Jump(p(-100, 50)), Step::Stitch(p(-100, 50)), Step::Stitch(p(-75, 50)), Step::Stitch(p(-50, 60))],
                },
            ],
        }
    }

    #[test]
    fn a_plan_round_trips_through_a_document() {
        let plan = sample();
        let doc = to_document(&plan);
        let back = from_document(&doc, &FromDocOptions { mode: Mode::Manual, ..Default::default() }).unwrap().plan;
        let stitches = |p: &StitchPlan| {
            p.blocks
                .iter()
                .map(|b| b.steps.iter().filter_map(|s| if let Step::Stitch(q) = s { Some(*q) } else { None }).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        assert_eq!(stitches(&back), stitches(&plan));
        assert_eq!(back.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>(), plan.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>());
        // Our own plans come back step for step, trims and jumps included.
        assert_eq!(back.blocks, plan.blocks);
    }

    #[test]
    fn the_document_has_one_layer_of_colour_groups_and_a_margin() {
        let doc = to_document(&sample());
        assert_eq!(doc.units, Unit::Millimeters);
        assert_eq!(doc.layers.len(), 1);
        let layer = &doc.layers[0];
        assert_eq!(layer.name.as_deref(), Some("Stitches"));
        let groups = layer.children().unwrap();
        assert_eq!(groups.iter().map(|g| g.name.clone().unwrap()).collect::<Vec<_>>(), vec!["Colour 1", "Colour 2"]);
        let runs = groups[0].children().unwrap();
        assert_eq!(runs.len(), 2);
        let NodeKind::Path { path, .. } = &runs[0].kind else { panic!("not a path") };
        assert!(!path.subpaths[0].closed);
        assert_eq!(path.subpaths[0].anchors.len(), 3);
        let stroke = runs[0].appearance.stroke().unwrap();
        assert!((stroke.width - THREAD_WIDTH_MM * PT_PER_MM).abs() < 1e-9);
        assert_eq!(stroke.paint.color().unwrap().to_rgba8(1.0), [200, 10, 20, 255]);
        assert!(runs[0].appearance.fill_paint().is_none());
        // Extents −10..32 mm by 0..31.5 mm, grown by 10 mm on each side.
        let pt = PT_PER_MM;
        let r = doc.artboards[0].rect;
        assert_eq!(doc.artboards.len(), 1);
        for (got, want) in [(r.x0, -20.0), (r.y0, -10.0), (r.x1, 42.0), (r.y1, 41.5)] {
            assert!((got - want * pt).abs() < 1e-9, "{got} != {want} mm");
        }
    }

    #[test]
    fn a_document_from_a_plan_is_marked_as_one() {
        let doc = to_document(&sample());
        assert!(is_stitch_document(&doc));
        let data = doc.layers[0].extras().attrs.as_deref().map(|a| a.data.clone()).unwrap_or_default();
        assert_eq!(data, vec![("stitchFile".to_string(), "1".to_string())]);
        assert!(!is_stitch_document(&vectorcraft_doc::Document::new(100.0, 100.0)));
        // Art drawn by hand is not: the mark is on the layer, not the paths.
        let mut other = to_document(&sample());
        let mut layer = (*other.layers[0]).clone();
        layer.edit_attrs(|a| a.data.clear());
        other.layers = vec![Arc::new(layer)];
        assert!(!is_stitch_document(&other));
    }

    #[test]
    fn an_empty_plan_gives_an_empty_layer() {
        let doc = to_document(&StitchPlan::default());
        assert_eq!(doc.layers.len(), 1);
        assert!(doc.layers[0].children().unwrap().is_empty());
        assert!(doc.artboards[0].rect.width() > 0.0);
    }
}
