//! Auto-digitize (E10): shapes become editable stitch objects chosen by their width
//! ([`vectorcraft_stitch::auto_shape`]): running stitch, two-rail satin columns and tatami fills.
//! `stitch.autoDigitize` replaces the selected shapes, `stitch.autoDigitize.info` says what they
//! would become, and `image.autoDigitize` traces an image for embroidery and digitizes every
//! region, then sorts the result by colour and joins it closest. Type goes through
//! `text.toStitches` instead.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, StitchProps, StitchType};
use vectorcraft_geom::PathData;
use vectorcraft_stitch::{
    AutoKind, AutoParams, COMPENSATION_MM, Column, MAX_STEPS, PT_PER_MM, StrokeKind, THREAD_WIDTH_MM, auto_shape, closest_join_ways,
    colour_sort_order, outline_mm, stroke_kind,
};
use vectorcraft_trace as tr;

use super::super::fileio::stitchfile;
use super::super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_doc, number_or, switch_or, targets};
use super::lettering::{stitch_object, subpath};

const AUTO: &str = "stitch.autoDigitize";
const INFO: &str = "stitch.autoDigitize.info";
const IMAGE: &str = "image.autoDigitize";

/// The range of `minColumn`, mm (the stitch property's).
const MIN_COLUMN_RANGE: (f64, f64) = (0.3, 3.0);
/// The range of `maxSatin`, mm.
const MAX_SATIN_RANGE: (f64, f64) = (2.0, 15.0);
/// The range of `colors` for an image.
const COLORS_RANGE: (u32, u32) = (2, 12);
/// The default number of colours an image is traced with.
const DEFAULT_COLORS: u32 = 6;
/// The default smallest region of an image, mm².
const DEFAULT_MIN_REGION_MM2: f64 = 2.0;
/// The most regions one image gives (more is refused: fewer colours or a bigger smallest region).
const MAX_REGIONS: usize = 2_000;
/// The name of the layer the digitized image is kept on.
const TEMPLATE_LAYER: &str = "Auto-Digitize Template";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stitch.autoDigitize.info",
            "Auto-Digitize Info",
            [],
            None,
            "{ids?|id? (default: the selection; groups give the paths inside them), minColumn?: 1 (mm, 0.3–3: no wider is running stitch), maxSatin?: 8 (mm, 2–15: wider is tatami fill)} what each selected path or compound path would become by stitch.autoDigitize, nothing changed → {objects: [{id, kind: run|satin|fill|mixed|none, columns, fills, runs}]}; objects with a stitch type of their own other than automatic (stitch objects already) and type are left out; none left is an error",
            has_doc,
            info
        ),
        cmd!(
            "stitch.autoDigitize",
            "Auto-Digitize Selection",
            ["Embroidery"],
            None,
            "{ids?|id? (default: the selection; groups give the paths inside them), minColumn?: 1 (mm, 0.3–3), maxSatin?: 8 (mm, 2–15), keepSource?: false (hide the source instead of removing it)} replace each path or compound path with the stitch objects its width calls for, a group named after it holding, in sewing order: a tatami fill of its wide part (the outline as drawn when it is all wide), two-rail satin columns (stitch type satin, compensation 0.2 mm, underlay) for stretches from minColumn to maxSatin wide, and running stitch lines for thinner ones, in its thread (the fill colour, else the stroke's); a stroke is classed by its weight (a run, a satin column that wide, or its outline filled). The group takes the source's place in the sewing order. Objects with a stitch type of their own and type (text.toStitches) are left alone. One undo step → {created: [group ids], columns, fills, runs}",
            has_doc,
            run
        ),
        cmd!(
            "image.autoDigitize",
            "Auto-Digitize Image",
            ["Embroidery"],
            None,
            "{id? (default: the selected image), colors?: 6 (2–12 thread colours), minRegionMm2?: 2 (mm², 0–100: smaller regions are dropped), minColumn?: 1, maxSatin?: 8 (mm, as stitch.autoDigitize), keepImage?: true (the image moves to a hidden template layer; false: it is removed)} trace the image for embroidery (Image Trace in Color mode with a limited palette, white left out, fills only) and digitize each region as stitch.autoDigitize does, a group per region in a group named Auto-Digitized; then the sewing order is sorted by colour and joined closest (stitch.sequence.sortByColor, stitch.join.closest). One undo step → {id (the group), regions, colors, columns, fills, runs, colorChanges, trims}",
            has_doc,
            image
        ),
    ]
}

/// One stitch object to make.
enum Part {
    Fill(Vec<Vec<(f64, f64)>>),
    Column(Column),
    Run(Vec<(f64, f64)>, bool),
    /// A path sewn as a satin column of the stroke weight (mm).
    Satin(Vec<(f64, f64)>, bool, f64),
}

/// What one source becomes.
struct Plan {
    kind: AutoKind,
    /// The parts with their thread, fills first.
    parts: Vec<(Part, [u8; 3])>,
}

impl Plan {
    fn count(&self) -> (usize, usize, usize) {
        let (mut c, mut f, mut r) = (0, 0, 0);
        for (p, _) in &self.parts {
            match p {
                Part::Fill(_) => f += 1,
                Part::Column(_) | Part::Satin(..) => c += 1,
                Part::Run(..) => r += 1,
            }
        }
        (c, f, r)
    }
}

/// The thresholds in `p`.
fn params(p: &Value, cmd: &str) -> Result<AutoParams> {
    let d = AutoParams::default();
    let min_column = number_or(p, "minColumn", d.min_column, cmd)?;
    let max_satin = number_or(p, "maxSatin", d.max_satin, cmd)?;
    let within = |v: f64, (lo, hi): (f64, f64)| v.is_finite() && (lo..=hi).contains(&v);
    if !within(min_column, MIN_COLUMN_RANGE) {
        return Err(bad(cmd, format!("minColumn must be {}–{} mm, not {min_column}", MIN_COLUMN_RANGE.0, MIN_COLUMN_RANGE.1)));
    }
    if !within(max_satin, MAX_SATIN_RANGE) {
        return Err(bad(cmd, format!("maxSatin must be {}–{} mm, not {max_satin}", MAX_SATIN_RANGE.0, MAX_SATIN_RANGE.1)));
    }
    if max_satin <= min_column {
        return Err(bad(cmd, "maxSatin must be wider than minColumn"));
    }
    Ok(AutoParams { min_column, max_satin })
}

/// The solid colour of `paint`, black for a gradient or pattern.
fn rgb_of(paint: &Paint) -> [u8; 3] {
    match paint {
        Paint::Solid { color, .. } => {
            let [r, g, b, _] = color.to_rgba8(1.0);
            [r, g, b]
        }
        _ => [0, 0, 0],
    }
}

/// Whether `n` is a shape auto-digitize works on: a visible path or compound path that is not a
/// guide or clipping path and has no stitch type of its own.
fn digitizable(n: &Node) -> bool {
    let shape = match &n.kind {
        NodeKind::Path { guide, clipping, .. } => !guide && !clipping,
        NodeKind::Compound { .. } => true,
        _ => false,
    };
    shape && n.stitch_props().is_none_or(|p| p.kind == StitchType::Auto)
}

/// The shapes among `ids` and inside the groups and layers they name, in paint order.
fn sources(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    fn walk(n: &Node, out: &mut Vec<NodeId>) {
        if !n.visible {
            return;
        }
        if digitizable(n) {
            out.push(n.id);
        } else if matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) {
            for c in n.children().into_iter().flatten() {
                walk(c, out);
            }
        }
    }
    let mut out = vec![];
    for id in ids {
        if let Some(n) = d.node(*id) {
            walk(n, &mut out);
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|i| seen.insert(*i));
    out
}

/// The parts closed outline `rings` (mm) becomes in thread `rgb`.
fn region_parts(rings: &[Vec<(f64, f64)>], rgb: [u8; 3], p: &AutoParams, out: &mut Vec<(Part, [u8; 3])>) -> std::result::Result<AutoKind, String> {
    let shape = auto_shape(rings, p)?;
    if !shape.fill.is_empty() {
        out.push((Part::Fill(shape.fill), rgb));
    }
    out.extend(shape.columns.into_iter().map(|c| (Part::Column(c), rgb)));
    out.extend(shape.runs.into_iter().map(|r| (Part::Run(r, false), rgb)));
    Ok(shape.kind)
}

/// The more telling of two kinds, for a source with a fill and a stroke.
fn join_kind(a: AutoKind, b: AutoKind) -> AutoKind {
    match (a, b) {
        (AutoKind::None, k) | (k, AutoKind::None) => k,
        (x, y) if x == y => x,
        _ => AutoKind::Mixed,
    }
}

/// What path or compound path `n` becomes: its fill classified as a region, its stroke by weight.
fn plan_of(n: &Node, p: &AutoParams) -> std::result::Result<Plan, String> {
    let fill = n.appearance.fill().filter(|f| f.visible && !f.paint.is_none());
    let stroke = n.appearance.stroke().filter(|s| s.visible && !s.paint.is_none() && s.width > 0.0);
    let lines = outline_mm(n, MAX_STEPS)?;
    let mut plan = Plan { kind: AutoKind::None, parts: vec![] };
    if let Some(f) = fill {
        let rings: Vec<Vec<(f64, f64)>> = lines.iter().filter(|l| l.1).map(|l| l.0.clone()).collect();
        if !rings.is_empty() {
            plan.kind = region_parts(&rings, rgb_of(&f.paint), p, &mut plan.parts)?;
        }
    }
    if let Some(s) = stroke {
        let rgb = rgb_of(&s.paint);
        let weight = s.width / PT_PER_MM;
        match stroke_kind(weight, p) {
            StrokeKind::Run => {
                plan.parts.extend(lines.iter().map(|(pts, closed)| (Part::Run(pts.clone(), *closed), rgb)));
                plan.kind = join_kind(plan.kind, AutoKind::Run);
            }
            StrokeKind::Satin => {
                plan.parts.extend(lines.iter().map(|(pts, closed)| (Part::Satin(pts.clone(), *closed, weight), rgb)));
                plan.kind = join_kind(plan.kind, AutoKind::Satin);
            }
            StrokeKind::Fill => {
                let path = collect_path(n);
                let outlined =
                    vectorcraft_pathops::outline_stroke(&path, s.width, vectorcraft_pathops::Cap::Butt, vectorcraft_pathops::Join::Round, 4.0);
                let tmp = Node::path(NodeId(0), outlined, Appearance::default());
                let rings: Vec<Vec<(f64, f64)>> = outline_mm(&tmp, MAX_STEPS)?.into_iter().filter(|l| l.1).map(|l| l.0).collect();
                let k = region_parts(&rings, rgb, p, &mut plan.parts)?;
                plan.kind = join_kind(plan.kind, k);
            }
        }
    }
    // Fills first, then columns, then runs.
    plan.parts.sort_by_key(|(part, _)| match part {
        Part::Fill(_) => 0,
        Part::Column(_) | Part::Satin(..) => 1,
        Part::Run(..) => 2,
    });
    Ok(plan)
}

/// The subpaths of a path or of a compound path's members, as one path.
fn collect_path(n: &Node) -> PathData {
    let mut out = PathData::default();
    match &n.kind {
        NodeKind::Path { path, .. } => out.subpaths.extend(path.subpaths.iter().cloned()),
        NodeKind::Compound { children, .. } => {
            for c in children {
                out.subpaths.extend(collect_path(c).subpaths);
            }
        }
        _ => {}
    }
    out
}

/// The stitch objects of `plan`, made in `d`.
fn make_parts(d: &mut Document, plan: Plan) -> Vec<Arc<Node>> {
    let mut out = vec![];
    for (part, [r, g, b]) in plan.parts {
        let thread = Paint::solid(Color::rgb8(r, g, b));
        let stroke = Appearance::basic(Paint::None, thread.clone(), THREAD_WIDTH_MM * PT_PER_MM);
        let n = match part {
            Part::Fill(rings) => {
                let props = StitchProps { kind: StitchType::Tatami, ..Default::default() };
                stitch_object(d, rings.iter().map(|r| subpath(r, true)).collect(), Appearance::basic(thread, Paint::None, 1.0), props)
            }
            Part::Column(c) => {
                let props = StitchProps { kind: StitchType::Satin, compensation: Some(COMPENSATION_MM), underlay: Some(true), ..Default::default() };
                stitch_object(d, vec![subpath(&c.left, false), subpath(&c.right, false)], stroke, props)
            }
            Part::Satin(pts, closed, width) => {
                let props = StitchProps { kind: StitchType::Satin, satin_width: Some(width), ..Default::default() };
                stitch_object(d, vec![subpath(&pts, closed)], Appearance::basic(Paint::None, thread, width * PT_PER_MM), props)
            }
            Part::Run(pts, closed) => {
                let props = StitchProps { kind: StitchType::Run, ..Default::default() };
                stitch_object(d, vec![subpath(&pts, closed)], stroke, props)
            }
        };
        out.push(Arc::new(n));
    }
    out
}

/// The shapes `p` names (or the selection) that auto-digitize works on.
fn shape_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids = targets(s, p)?;
    for id in &ids {
        d.node(*id).ok_or_else(|| bad(cmd, format!("no object {}", id.0)))?;
    }
    let found = sources(d, &ids);
    if found.is_empty() {
        return Err(bad(cmd, "select paths or compound paths that are not stitch objects yet (type: Convert Type to Stitch Objects)"));
    }
    Ok(found)
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let ap = params(p, INFO)?;
    let ids = shape_targets(s, p, INFO)?;
    let d = &s.doc()?.doc;
    let mut objects = vec![];
    for id in ids {
        let Some(n) = d.node(id) else { continue };
        let plan = plan_of(n, &ap).map_err(|e| bad(INFO, e))?;
        let (columns, fills, runs) = plan.count();
        objects.push(json!({"id": id.0, "kind": plan.kind.id(), "columns": columns, "fills": fills, "runs": runs}));
    }
    Ok(json!({ "objects": objects }))
}

/// Replace `old` in the sewing order with `new`, where it is listed.
pub(super) fn replace_in_order(d: &mut Document, old: NodeId, new: &[NodeId]) {
    if let Some(at) = d.stitch.order.iter().position(|i| *i == old) {
        d.stitch.order.splice(at..=at, new.iter().copied());
    }
}

/// The ids of the stitch objects in `nodes`.
fn leaf_ids(nodes: &[Arc<Node>]) -> Vec<NodeId> {
    nodes.iter().map(|n| n.id).collect()
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let ap = params(p, AUTO)?;
    let keep = switch_or(p, "keepSource", false, AUTO)?;
    let ids = shape_targets(s, p, AUTO)?;
    let mut plans = vec![];
    {
        let d = &s.doc()?.doc;
        for id in ids {
            let Some(n) = d.node(id) else { continue };
            let plan = plan_of(n, &ap).map_err(|e| bad(AUTO, e))?;
            if !plan.parts.is_empty() {
                plans.push((id, n.display_name(), plan));
            }
        }
    }
    if plans.is_empty() {
        return Err(bad(AUTO, "the selected shapes have nothing to sew (no fill or stroke, or no area)"));
    }
    let (mut columns, mut fills, mut runs) = (0usize, 0usize, 0usize);
    for (_, _, plan) in &plans {
        let (c, f, r) = plan.count();
        columns += c;
        fills += f;
        runs += r;
    }
    let created = s.edit("Auto-Digitize", |d, sel| {
        let mut out = vec![];
        for (id, name, plan) in plans {
            let Some((parent, index, _)) = d.position(id) else { continue };
            let parts = make_parts(d, plan);
            let leaves = leaf_ids(&parts);
            let gid = d.alloc_id();
            let mut g = Node::group(gid, parts);
            g.name = Some(format!("{name} (stitches)"));
            if keep {
                if let Some(n) = d.node_mut(id) {
                    n.visible = false;
                }
                d.insert(parent, index + 1, g)?;
            } else {
                d.remove(id)?;
                d.insert(parent, index, g)?;
            }
            replace_in_order(d, id, &leaves);
            out.push(gid);
        }
        if out.is_empty() {
            return Err(EngineError::Other("Auto-Digitize: nothing was made".into()));
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "created": created.iter().map(|i| i.0).collect::<Vec<_>>(), "columns": columns, "fills": fills, "runs": runs }))
}

/// The image `p` names, else the selected one.
fn image_target(s: &Session, p: &Value) -> Result<(NodeId, Box<vectorcraft_doc::ImageObject>)> {
    let d = &s.doc()?.doc;
    let ids = targets(s, p)?;
    ids.iter()
        .find_map(|id| match &d.node(*id)?.kind {
            NodeKind::Image(i) => Some((*id, i.clone())),
            _ => None,
        })
        .ok_or_else(|| bad(IMAGE, "select an image"))
}

fn image(s: &mut Session, p: &Value) -> Result<Value> {
    let ap = params(p, IMAGE)?;
    let colors = number_or(p, "colors", f64::from(DEFAULT_COLORS), IMAGE)?;
    if !(colors.fract() == 0.0 && (f64::from(COLORS_RANGE.0)..=f64::from(COLORS_RANGE.1)).contains(&colors)) {
        return Err(bad(IMAGE, format!("colors must be a whole number {}–{}, not {colors}", COLORS_RANGE.0, COLORS_RANGE.1)));
    }
    let min_region = number_or(p, "minRegionMm2", DEFAULT_MIN_REGION_MM2, IMAGE)?;
    if !(min_region.is_finite() && (0.0..=100.0).contains(&min_region)) {
        return Err(bad(IMAGE, format!("minRegionMm2 must be 0–100, not {min_region}")));
    }
    let keep = switch_or(p, "keepImage", true, IMAGE)?;
    let (target, img) = image_target(s, p)?;
    let bytes = {
        let d = &s.doc()?.doc;
        let blob = d.images.get(&img.key).ok_or_else(|| EngineError::Other(format!("image data `{}` is missing", img.key)))?;
        Arc::clone(&blob.bytes)
    };
    let raster = tr::Raster::decode_to_fit(&bytes).map_err(|e| bad(IMAGE, e.to_string()))?;
    // Raster pixels → the image's pixels → document.
    let sx = f64::from(img.width.max(1)) / f64::from(raster.width.max(1));
    let sy = f64::from(img.height.max(1)) / f64::from(raster.height.max(1));
    let xf = img.xf * vectorcraft_geom::Affine::scale_non_uniform(sx, sy);
    let px_mm = xf.determinant().abs().sqrt() / PT_PER_MM;
    if !(px_mm.is_finite() && px_mm > 0.0) {
        return Err(bad(IMAGE, "the image has no size on the page"));
    }
    // An embroidery preset: few flat colours, no strokes, no white, no specks.
    let trace = tr::TraceParams {
        mode: tr::Mode::Color,
        palette: tr::Palette::Limited,
        // Checked to 2–12 above.
        colors: colors as u32,
        ignore_white: true,
        fills: true,
        strokes: false,
        noise: (min_region / (px_mm * px_mm)).round().clamp(1.0, 1e6) as u32,
        ..tr::TraceParams::default()
    };
    let res = tr::trace_within(&raster, &trace, super::super::buildcmds::MAX_TRACE_ANCHORS).map_err(|e| bad(IMAGE, e.to_string()))?;
    if res.paths.len() > MAX_REGIONS {
        return Err(bad(
            IMAGE,
            format!("the image traces to {} regions, over {MAX_REGIONS}: use fewer colours or a bigger smallest region", res.paths.len()),
        ));
    }
    let mut plans = vec![];
    let (mut columns, mut fills, mut runs) = (0usize, 0usize, 0usize);
    for tp in &res.paths {
        if tp.stroke.is_some() || tp.pixels as f64 * px_mm * px_mm < min_region {
            continue;
        }
        let tmp = Node::path(NodeId(0), tp.path.transformed(xf), Appearance::default());
        let rings: Vec<Vec<(f64, f64)>> = outline_mm(&tmp, MAX_STEPS).map_err(|e| bad(IMAGE, e))?.into_iter().filter(|l| l.1).map(|l| l.0).collect();
        let mut plan = Plan { kind: AutoKind::None, parts: vec![] };
        plan.kind = region_parts(&rings, tp.color, &ap, &mut plan.parts).map_err(|e| bad(IMAGE, e))?;
        if plan.parts.is_empty() {
            continue;
        }
        let (c, f, r) = plan.count();
        columns += c;
        fills += f;
        runs += r;
        plans.push(plan);
    }
    if plans.is_empty() {
        return Err(bad(IMAGE, "the image has no regions to sew (all white or smaller than minRegionMm2)"));
    }
    let regions = plans.len();
    let options = stitchfile::from_doc_options(IMAGE, &json!({}))?;
    let gid = s.edit("Auto-Digitize Image", |d, sel| {
        let Some((parent, index, _)) = d.position(target) else { return Err(EngineError::NoNode(target)) };
        let mut groups = vec![];
        for plan in plans {
            let parts = make_parts(d, plan);
            groups.push(Arc::new(Node::group(d.alloc_id(), parts)));
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, groups);
        g.name = Some("Auto-Digitized".into());
        let image = d.remove(target)?;
        d.insert(parent, index, g)?;
        if keep {
            let layer = d.add_layer(Some(TEMPLATE_LAYER));
            d.move_node(layer, None, 0)?;
            if let Some(n) = d.node_mut(layer) {
                n.locked = true;
                n.visible = false;
                if let NodeKind::Layer { template, .. } = &mut n.kind {
                    *template = true;
                }
            }
            d.insert(Some(layer), 0, (*image).clone())?;
        }
        // Sort by colour, then the closest join (the sequencing pass's own functions).
        let seq = vectorcraft_stitch::sequence(d, &options).map_err(EngineError::Other)?;
        d.stitch.order = colour_sort_order(&seq);
        let seq = vectorcraft_stitch::sequence(d, &options).map_err(EngineError::Other)?;
        for (id, frac) in closest_join_ways(d, &seq) {
            if let Some(n) = d.node_mut(id) {
                let mut props = n.stitch_props().copied().unwrap_or_default();
                props.join_entry = frac.0;
                props.join_exit = frac.1;
                n.set_stitch_props(props);
            }
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    let (plan, _) = stitchfile::plan(IMAGE, &s.doc()?.doc, &json!({}), "dst", None)?;
    let st = vectorcraft_stitch::stats(&plan);
    Ok(json!({
        "id": gid.0, "regions": regions, "colors": res.palette.len(), "columns": columns, "fills": fills, "runs": runs,
        "colorChanges": st.color_changes, "trims": st.trims,
    }))
}

#[cfg(test)]
mod tests;
