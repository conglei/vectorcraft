//! Stitches from a document: stroked paths become running stitch.

use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{AnchorKind, BezPath, PathEl, SubPath};

use crate::run::run_stitches;
use crate::{Block, MAX_STEPS, Pt, Step, StitchPlan, Thread};

/// Millimetres per document point.
const MM_PER_PT: f64 = 25.4 / 72.0;
/// How far a flattened curve may stray from the true one, in millimetres.
const FLATTEN_TOLERANCE_MM: f64 = 0.05;

/// How a path becomes stitches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Running stitch along the outline, stepped at the stitch length.
    #[default]
    Run,
    /// One needle point per anchor; curves are ignored.
    Manual,
}

/// Options for [`from_document`].
#[derive(Clone, Debug)]
pub struct FromDocOptions {
    pub mode: Mode,
    /// Running stitch length, in plan units (0.1 mm).
    pub stitch_length: i32,
    /// Runs that start within this distance (plan units) of where the last one ended are joined
    /// without a trim.
    pub min_stitch: i32,
    /// Stitch only these objects and what they contain; `None`: the whole document.
    pub only: Option<Vec<NodeId>>,
}

impl Default for FromDocOptions {
    fn default() -> Self {
        Self { mode: Mode::Run, stitch_length: 25, min_stitch: 3, only: None }
    }
}

/// A plan made from a document, with what the user should be told about it.
#[derive(Debug)]
pub struct Planned {
    pub plan: StitchPlan,
    pub warnings: Vec<String>,
}

/// The stitch plan for `doc`'s visible stroked paths and compound paths, bottom to top. Each
/// subpath is one run; consecutive objects of one thread colour share a block.
pub fn from_document(doc: &Document, o: &FromDocOptions) -> Result<Planned, String> {
    let mut w = Walk { o, blocks: vec![], pos: None, steps: 0, filled: 0, other: 0, patterned: false };
    // `doc.layers` is stored bottom first, the order the renderer paints them in.
    for layer in &doc.layers {
        w.node(layer, false)?;
    }
    if w.blocks.is_empty() {
        return Err("nothing to stitch: the document has no stroked paths".into());
    }
    let mut warnings = vec![];
    if w.filled > 0 {
        warnings.push(format!("{} filled objects without a stroke were not stitched", w.filled));
    }
    if w.other > 0 {
        warnings.push(format!("{} objects of other kinds (type, images, symbols) were not stitched", w.other));
    }
    if w.patterned {
        warnings.push("a stroke with a gradient or pattern was stitched in black".into());
    }
    let plan = StitchPlan { name: String::new(), blocks: w.blocks };
    plan.check()?;
    Ok(Planned { plan, warnings })
}

struct Walk<'o> {
    o: &'o FromDocOptions,
    blocks: Vec<Block>,
    /// Where the needle is: the last stitch made.
    pos: Option<Pt>,
    steps: usize,
    filled: usize,
    other: usize,
    patterned: bool,
}

impl Walk<'_> {
    /// Stitch `n` and what it holds. `listed`: an ancestor is in [`FromDocOptions::only`].
    /// Recursion is bounded by the document's own nesting limit.
    fn node(&mut self, n: &Node, listed: bool) -> Result<(), String> {
        if !n.visible {
            return Ok(());
        }
        let listed = listed || self.o.only.as_ref().is_none_or(|ids| ids.contains(&n.id));
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
                for c in children {
                    self.node(c, listed)?;
                }
            }
            _ if !listed => {}
            // Guides don't print, so they aren't sewn either.
            NodeKind::Path { guide: true, .. } => {}
            NodeKind::Path { .. } | NodeKind::Compound { .. } => self.shape(n)?,
            _ => self.other += 1,
        }
        Ok(())
    }

    fn shape(&mut self, n: &Node) -> Result<(), String> {
        let stroke = n.appearance.stroke().filter(|s| s.visible).map(|s| &s.paint);
        let rgb = match stroke {
            None | Some(Paint::None) => {
                if !n.appearance.fill_paint().is_none() {
                    self.filled += 1;
                }
                return Ok(());
            }
            Some(Paint::Solid { color, .. }) => {
                let [r, g, b, _] = color.to_rgba8(1.0);
                [r, g, b]
            }
            Some(Paint::Gradient(_) | Paint::Pattern { .. }) => {
                self.patterned = true;
                [0, 0, 0]
            }
        };
        let mut subpaths: Vec<&SubPath> = vec![];
        collect_subpaths(n, &mut subpaths);
        for sp in subpaths {
            let run = match self.o.mode {
                Mode::Run => run_of(sp, self.o.stitch_length)?,
                Mode::Manual => manual_of(sp)?,
            };
            self.add_run(rgb, &run)?;
        }
        Ok(())
    }

    /// Sew `run` in thread `rgb`: a new block at a colour change (which starts with a jump to the
    /// run, except at the very start), otherwise a trim and a jump unless the run starts within
    /// `min_stitch` of the needle.
    fn add_run(&mut self, rgb: [u8; 3], run: &[Pt]) -> Result<(), String> {
        let Some(&start) = run.first() else { return Ok(()) };
        let mut skip_first = false;
        let mut lead: Vec<Step> = vec![];
        match self.blocks.last() {
            Some(b) if b.thread.rgb == rgb => {
                let near = self.pos.is_some_and(|p| p.dist(start) <= f64::from(self.o.min_stitch));
                if near {
                    // In run mode a first stitch where the needle already is would sew nothing.
                    skip_first = self.o.mode == Mode::Run && self.pos == Some(start);
                } else {
                    lead = vec![Step::Trim, Step::Jump(start)];
                }
            }
            last => {
                if last.is_some() {
                    lead.push(Step::Jump(start));
                }
                self.blocks.push(Block { thread: Thread { rgb, name: None, code: None }, steps: vec![] });
            }
        }
        let added = lead.len() + run.len();
        self.steps = self.steps.saturating_add(added);
        if self.steps > MAX_STEPS {
            return Err(format!("the design has more than {MAX_STEPS} stitches"));
        }
        let Some(block) = self.blocks.last_mut() else { return Ok(()) };
        block.steps.extend(lead);
        block.steps.extend(run.iter().skip(usize::from(skip_first)).map(|p| Step::Stitch(*p)));
        self.pos = run.last().copied();
        Ok(())
    }
}

/// The subpaths of a path, or of a compound path's member paths (in their order).
fn collect_subpaths<'n>(n: &'n Node, out: &mut Vec<&'n SubPath>) {
    match &n.kind {
        NodeKind::Path { path, .. } => out.extend(path.subpaths.iter()),
        NodeKind::Compound { children, .. } => {
            for c in children {
                collect_subpaths(c, out);
            }
        }
        _ => {}
    }
}

fn mm(p: vectorcraft_geom::Point) -> Result<(f64, f64), String> {
    let q = (p.x * MM_PER_PT, p.y * MM_PER_PT);
    match Pt::from_mm(q.0, q.1) {
        Some(_) => Ok(q),
        None => Err("the art lies too far from the origin to stitch".into()),
    }
}

/// Running stitch along `sp`, flattened, with its corner anchors kept as corners.
fn run_of(sp: &SubPath, stitch_length: i32) -> Result<Vec<Pt>, String> {
    let Some(first) = sp.anchors.first() else { return Ok(vec![]) };
    let mut points = vec![mm(first.p)?];
    let mut corners = vec![first.kind == AnchorKind::Corner];
    let tolerance = FLATTEN_TOLERANCE_MM / MM_PER_PT;
    let count = sp.segment_count();
    for i in 0..count {
        let Some(end) = sp.anchors.get((i + 1) % sp.anchors.len().max(1)) else { break };
        let c = sp.segment(i);
        if sp.segment_is_line(i) {
            points.push(mm(c.p3)?);
            corners.push(end.kind == AnchorKind::Corner);
            continue;
        }
        let mut bp = BezPath::new();
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
        let mut flat = vec![];
        vectorcraft_geom::flatten(bp.elements().iter().copied(), tolerance, |el| {
            if let PathEl::LineTo(p) = el {
                flat.push(p);
            }
        });
        // The curve's own end is the anchor, whatever the flattening ended on.
        flat.pop();
        for p in flat {
            points.push(mm(p)?);
            corners.push(false);
        }
        points.push(mm(c.p3)?);
        corners.push(end.kind == AnchorKind::Corner);
    }
    if sp.closed && count > 0 {
        // The closing segment ends back on the first anchor, which `closed` adds below.
        points.pop();
        corners.pop();
    }
    Ok(run_stitches(&points, &corners, sp.closed && count > 0, stitch_length))
}

/// One needle point per anchor; a closed subpath returns to its first anchor.
fn manual_of(sp: &SubPath) -> Result<Vec<Pt>, String> {
    let mut out = Vec::with_capacity(sp.anchors.len().saturating_add(1));
    for a in &sp.anchors {
        let (x, y) = mm(a.p)?;
        if let Some(p) = Pt::from_mm(x, y) {
            out.push(p);
        }
    }
    if sp.closed
        && let Some(&first) = out.first()
    {
        out.push(first);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId};
    use vectorcraft_geom::{Anchor, PathData, Point, SubPath};

    use super::*;
    use crate::{Pt, Step};

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const PT_PER_MM: f64 = 72.0 / 25.4;

    fn mm(x: f64, y: f64) -> Point {
        Point::new(x * PT_PER_MM, y * PT_PER_MM)
    }

    fn square_mm(side: f64) -> PathData {
        PathData::single(SubPath::polyline(&[mm(0.0, 0.0), mm(side, 0.0), mm(side, side), mm(0.0, side)], true))
    }

    fn line_mm(x0: f64, y0: f64, x1: f64, y1: f64) -> PathData {
        PathData::single(SubPath::polyline(&[mm(x0, y0), mm(x1, y1)], false))
    }

    fn single_anchor() -> PathData {
        PathData::single(SubPath::new(vec![Anchor::corner(mm(1.0, 1.0))], false))
    }

    fn stroked(path: PathData, rgb: [u8; 3]) -> (PathData, Appearance) {
        (path, Appearance::basic(Paint::None, Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])), 1.0))
    }

    /// A document whose one layer holds `art` as paths, bottom first.
    fn doc_with(art: Vec<(PathData, Appearance)>) -> Document {
        let mut doc = Document::new(500.0, 500.0);
        let mut layer = Node::layer(doc.alloc_id(), "Art", LayerColor::Preset(0));
        let children: Vec<Arc<Node>> = art.into_iter().map(|(p, a)| Arc::new(Node::path(doc.alloc_id(), p, a))).collect();
        if let Some(c) = layer.children_mut() {
            *c = children;
        }
        doc.layers = vec![Arc::new(layer)];
        doc
    }

    /// A filled square with no stroke, a hidden stroked line, and a visible stroked line inside a
    /// group.
    fn mixed_doc() -> Document {
        let mut doc = Document::new(500.0, 500.0);
        let filled = Node::path(doc.alloc_id(), square_mm(10.0), Appearance::basic(Paint::solid(Color::rgb8(0, 255, 0)), Paint::None, 0.0));
        let (p, a) = stroked(line_mm(0.0, 20.0, 10.0, 20.0), BLUE);
        let mut hidden = Node::path(doc.alloc_id(), p, a);
        hidden.visible = false;
        let (p, a) = stroked(line_mm(0.0, 40.0, 10.0, 40.0), RED);
        let visible = Node::path(doc.alloc_id(), p, a);
        let group = Node::group(doc.alloc_id(), vec![Arc::new(hidden), Arc::new(visible)]);
        let mut layer = Node::layer(doc.alloc_id(), "Art", LayerColor::Preset(0));
        if let Some(c) = layer.children_mut() {
            *c = vec![Arc::new(filled), Arc::new(group)];
        }
        doc.layers = vec![Arc::new(layer)];
        doc
    }

    #[test]
    fn closed_path_returns_to_its_start() {
        let plan = from_document(&doc_with(vec![stroked(square_mm(10.0), RED)]), &FromDocOptions::default()).unwrap().plan;
        let pts: Vec<Pt> = plan.points().collect();
        assert_eq!(pts.first(), pts.last());
        assert_eq!(pts.len(), 17); // 4 sides × 4 steps of 2.5 mm, plus the start
    }

    #[test]
    fn colours_make_blocks_and_objects_are_trimmed_apart() {
        let d = doc_with(vec![
            stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED),
            stroked(line_mm(0.0, 20.0, 10.0, 20.0), RED),
            stroked(line_mm(0.0, 40.0, 10.0, 40.0), BLUE),
        ]);
        let plan = from_document(&d, &FromDocOptions::default()).unwrap().plan;
        assert_eq!(plan.blocks.len(), 2);
        assert_eq!(plan.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1);
        assert_eq!(plan.blocks[0].thread.rgb, [255, 0, 0]);
        // The first block starts with its first stitch; the second, at a colour change, with a
        // jump to its first point and no trim.
        assert_eq!(plan.blocks[0].steps.first(), Some(&Step::Stitch(Pt { x: 0, y: 0 })));
        assert_eq!(plan.blocks[1].steps.first(), Some(&Step::Jump(Pt { x: 0, y: 400 })));
        assert!(!plan.blocks[1].steps.contains(&Step::Trim));
    }

    #[test]
    fn fills_hidden_art_and_other_kinds_are_skipped_with_warnings() {
        // a filled square with no stroke, a hidden stroked line, a visible stroked line
        let out = from_document(&mixed_doc(), &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.len(), 1);
        assert_eq!(out.warnings, vec!["1 filled objects without a stroke were not stitched".to_string()]);
    }

    #[test]
    fn degenerate_paths_add_nothing() {
        let d = doc_with(vec![stroked(single_anchor(), RED), stroked(line_mm(5.0, 5.0, 5.0, 5.0), RED)]);
        assert_eq!(from_document(&d, &FromDocOptions::default()).unwrap_err(), "nothing to stitch: the document has no stroked paths");
    }

    #[test]
    fn manual_mode_uses_the_anchors() {
        let o = FromDocOptions { mode: Mode::Manual, ..Default::default() };
        let plan = from_document(&doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]), &o).unwrap().plan;
        assert_eq!(plan.blocks[0].steps, vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 100, y: 0 })]);
    }

    #[test]
    fn only_the_listed_objects_and_their_contents_are_stitched() {
        let d = doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED), stroked(line_mm(0.0, 20.0, 10.0, 20.0), BLUE)]);
        let second = d.layers[0].children().unwrap()[1].id;
        let plan = from_document(&d, &FromDocOptions { only: Some(vec![second]), ..Default::default() }).unwrap().plan;
        assert_eq!(plan.blocks.len(), 1);
        assert_eq!(plan.blocks[0].thread.rgb, BLUE);
        let layer = d.layers[0].id;
        let all = from_document(&d, &FromDocOptions { only: Some(vec![layer]), ..Default::default() }).unwrap().plan;
        assert_eq!(all.blocks.len(), 2);
        let none = from_document(&d, &FromDocOptions { only: Some(vec![NodeId(9999)]), ..Default::default() });
        assert!(none.is_err());
    }

    #[test]
    fn a_gradient_stroke_is_stitched_in_black_with_one_warning() {
        let gradient = |doc: &mut Document, y: f64| {
            let mut a = Appearance::basic(Paint::None, Paint::None, 1.0);
            if let Some(s) = a.stroke_mut() {
                s.paint = Paint::Pattern { pattern: "dots".into(), xf: Default::default() };
            }
            Arc::new(Node::path(doc.alloc_id(), line_mm(0.0, y, 10.0, y), a))
        };
        let mut d = doc_with(vec![]);
        let children = vec![gradient(&mut d, 0.0), gradient(&mut d, 20.0)];
        let mut layer = (*d.layers[0]).clone();
        *layer.children_mut().unwrap() = children;
        d.layers = vec![Arc::new(layer)];
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.len(), 1);
        assert_eq!(out.plan.blocks[0].thread.rgb, [0, 0, 0]);
        assert_eq!(out.warnings, vec!["a stroke with a gradient or pattern was stitched in black".to_string()]);
    }

    #[test]
    fn other_kinds_are_counted_in_a_warning() {
        let mut d = doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]);
        let symbol = Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "s".into(), xf: Default::default() }));
        let mut layer = (*d.layers[0]).clone();
        layer.children_mut().unwrap().push(symbol);
        d.layers = vec![Arc::new(layer)];
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.warnings, vec!["1 objects of other kinds (type, images, symbols) were not stitched".to_string()]);
    }

    #[test]
    fn a_curve_is_followed_within_tolerance() {
        // A quarter circle of radius 10 mm about the origin, from (10, 0) to (0, 10).
        let k = 0.552_284_749_8 * 10.0;
        let sp = SubPath::new(
            vec![Anchor::with_handles(mm(10.0, 0.0), mm(10.0, 0.0), mm(10.0, k)), Anchor::with_handles(mm(0.0, 10.0), mm(k, 10.0), mm(0.0, 10.0))],
            false,
        );
        let plan = from_document(&doc_with(vec![stroked(PathData::single(sp), RED)]), &FromDocOptions::default()).unwrap().plan;
        let pts: Vec<Pt> = plan.points().collect();
        assert_eq!(pts.first(), Some(&Pt { x: 100, y: 0 }));
        assert_eq!(pts.last(), Some(&Pt { x: 0, y: 100 }));
        // Arc length 15.7 mm at 2.5 mm: 7 steps.
        assert_eq!(pts.len(), 8);
        for p in &pts {
            let r = f64::from(p.x).hypot(f64::from(p.y));
            assert!((r - 100.0).abs() < 1.5, "{p:?} is {r} units from the centre");
        }
    }
}
