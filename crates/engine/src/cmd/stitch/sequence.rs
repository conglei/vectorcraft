//! Embroidery › Stitch Sequence: the order objects are sewn in and where each is entered and left
//! ([`vectorcraft_doc::StitchDesign`], [`vectorcraft_doc::StitchProps::entry`]). The order,
//! closest join, sort by colour, entry and exit points and the design's start and end; each change
//! is one undo step. Exports, `stitch.info` and the stitch preview sew by them
//! ([`vectorcraft_stitch::from_document`]).

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, MAX_COORD, NodeId, NodeKind, StitchType};
use vectorcraft_geom::Point;
use vectorcraft_stitch::sequence::{Cost, Item, closest_join, colour_changes, colour_sort, cost};
use vectorcraft_stitch::{FromDocOptions, PT_PER_MM, Pt, Sequenced, UNITS_PER_MM, inherited_props, outline_fraction, sequence, sewn_as};

use super::super::fileio::stitchfile;
use super::super::{CommandSpec, Result, Session, bad, cmd, has_doc, targets};

const LIST: &str = "stitch.sequence.list";
const MOVE: &str = "stitch.sequence.move";
const SET: &str = "stitch.sequence.set";
const SORT: &str = "stitch.sequence.sortByColor";
const ENTRY_EXIT: &str = "stitch.entryExit.set";
const JOIN: &str = "stitch.join.closest";
const START_END: &str = "stitch.designStartEnd.set";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stitch.sequence.list",
            "Stitch Sequence",
            [],
            None,
            "{options?: {…the stitch options of stitch.info (stitchLength, minStitch, mode, fill, stroke…); default: the export's}} the order the objects are sewn in (the order set with stitch.sequence.set, move or sortByColor, then every other sewn path and compound path in document order, bottom to top) → {objects: [{id, name, colors: [\"#rrggbb\"…] (the threads it sews in, in order), sews: [tatami|run|satin|manual…], stitches (needle points, before lock stitches), entry: {x, y}, exit: {x, y} (where it is entered and left, document units), own: {entry?, exit?, joinEntry?, joinExit?} (the points stored on it: fractions 0–1 along its outline, set by stitch.entryExit.set and stitch.join.closest), pinned (the user set its entry or exit), opened (on the layer an opened stitch file made: sewn as the file had it)}], colorBlocks: [{rgb: \"#rrggbb\", objects: [id…]}] (consecutive objects in one thread), colorChanges, trims, travelMm (of joining the objects in this order; stitch.info has the machine file's own), ordered (an order is set), start: {x, y}|null, end: {x, y}|null (the design's start and end)}",
            has_doc,
            list
        ),
        cmd!(
            "stitch.sequence.move",
            "Move in Stitch Sequence",
            [],
            None,
            "{id, to: index in the sewing order (0: first; past the end: last)} move one sewn object in the sewing order (the whole order is stored, so objects drawn later are sewn after it); one undo step → {order: [id…]}",
            has_doc,
            move_cmd
        ),
        cmd!(
            "stitch.sequence.set",
            "Set Stitch Sequence",
            [],
            None,
            "{ids: [id…] (sewn paths and compound paths, each once; []: document order again)} sew these first, in this order, then every other sewn object in document order; one undo step → {order: [id…]}",
            has_doc,
            set_cmd
        ),
        cmd!(
            "stitch.sequence.sortByColor",
            "Sort Sequence by Color",
            ["Embroidery"],
            None,
            "{options?} group the sewing order by thread colour: the colours in the order they are first sewn, the objects of each colour in their order; kept only when it lowers the colour changes (it never raises them; overlapping objects of different colours are not checked yet); one undo step when it changes the order → {changed, order: [id…], colorChanges: {before, after}}",
            has_doc,
            sort_cmd
        ),
        cmd!(
            "stitch.entryExit.set",
            "Set Entry and Exit Points",
            [],
            None,
            "{ids?|id? (default: the selection; a compound path's member stands for its compound path), entry?: a fraction 0–1 along the outline (by arc length, subpaths one after another) | {x, y} (document units: the nearest outline point) | null (unset), exit?: (the same), clear?: false (remove the entry and exit points, the user's and closest join's)} where sewing enters and leaves paths and compound paths (a closed outline starts at its entry, an open one is sewn from the end nearer it, a fill or satin is sewn forwards or backwards); the points move with the object; closest join keeps them; one undo step → {changed: [id…]}",
            has_doc,
            entry_exit
        ),
        cmd!(
            "stitch.join.closest",
            "Closest Join",
            ["Embroidery"],
            None,
            "{options?} in the current sewing order, enter each object nearest where the one before it was left (the first nearest the design start, the last left nearest the design end): open outlines and fills are turned, closed outlines started elsewhere, choosing the fewest trims, then the least travel, over the whole order at once; never more trims or travel than before; objects whose entry or exit the user set keep it; stores the chosen points on the objects (joinEntry, joinExit); one undo step when anything changes → {changed: [id…], before: {trims, travelMm, colorChanges}, after: {…}}",
            has_doc,
            join_cmd
        ),
        cmd!(
            "stitch.designStartEnd.set",
            "Set Design Start and End",
            [],
            None,
            "{start?: {x, y} | [x, y] (document units) | null (unset), end?: (the same)} where the machine starts (the first object is entered nearest it; a stitch file's origin, in place of the export's start option) and where the design ends (the last object is left nearest it); what is not given stays; one undo step → {start, end}",
            has_doc,
            start_end
        ),
    ]
}

/// The stitch options in `p`'s `options` (the export's when not given).
fn options(p: &Value, cmd: &str) -> Result<FromDocOptions> {
    let options = match p.get("options") {
        None | Some(Value::Null) => json!({}),
        Some(o) if o.is_object() => o.clone(),
        Some(o) => return Err(bad(cmd, format!("options must be an object of stitch options, not {o}"))),
    };
    stitchfile::from_doc_options(cmd, &options)
}

/// The document's sewing order with options `p`.
fn sequenced(s: &Session, p: &Value, cmd: &str) -> Result<Sequenced> {
    let o = options(p, cmd)?;
    sequence(&s.doc()?.doc, &o).map_err(|e| bad(cmd, e))
}

/// A plan point in document units, as `{x, y}`.
fn doc_xy(p: Pt) -> Value {
    let (x, y) = p.to_mm();
    let r = |v: f64| (v * PT_PER_MM * 1000.0).round() / 1000.0;
    json!({"x": r(x), "y": r(y)})
}

fn hex(rgb: [u8; 3]) -> String {
    let [r, g, b] = rgb;
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn items(s: &Sequenced) -> Vec<Item> {
    s.items.iter().map(|i| i.item.clone()).collect()
}

fn cost_json(c: &Cost) -> Value {
    json!({"trims": c.trims, "travelMm": (c.travel / UNITS_PER_MM * 10.0).round() / 10.0, "colorChanges": c.changes})
}

/// The name an object shows in the list: its own, else its kind.
fn name_of(d: &Document, id: NodeId) -> String {
    let Some(n) = d.node(id) else { return String::new() };
    match (&n.name, &n.kind) {
        (Some(name), _) if !name.trim().is_empty() => name.clone(),
        (_, NodeKind::Compound { .. }) => "<Compound Path>".into(),
        _ => "<Path>".into(),
    }
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let o = options(p, LIST)?;
    let seq = sequenced(s, p, LIST)?;
    let d = &s.doc()?.doc;
    let mut objects = vec![];
    let mut blocks: Vec<([u8; 3], Vec<u64>)> = vec![];
    for it in &seq.items {
        let n = d.node(it.id);
        let own = n.and_then(|n| n.stitch_props()).copied().unwrap_or_default();
        let sews: Vec<&str> = match (n, inherited_props(d, it.id)) {
            (Some(n), Some((props, opened))) => sewn_as(n, &props, opened, &o).into_iter().map(StitchType::id).collect(),
            _ => vec![],
        };
        let mut points = Map::new();
        for (k, v) in [("entry", own.entry_at()), ("exit", own.exit_at()), ("joinEntry", own.join_entry_at()), ("joinExit", own.join_exit_at())] {
            if let Some(v) = v {
                points.insert(k.into(), json!(v));
            }
        }
        let (entry, exit) = it.ends().map_or((Value::Null, Value::Null), |(a, b)| (doc_xy(a), doc_xy(b)));
        objects.push(json!({
            "id": it.id.0,
            "name": name_of(d, it.id),
            "colors": it.colours.iter().map(|c| hex(*c)).collect::<Vec<_>>(),
            "sews": sews,
            "stitches": it.stitches,
            "entry": entry,
            "exit": exit,
            "own": points,
            "pinned": it.pinned,
            "opened": it.opened,
        }));
        for c in &it.colours {
            match blocks.last_mut() {
                Some((rgb, ids)) if rgb == c => {
                    if ids.last() != Some(&it.id.0) {
                        ids.push(it.id.0);
                    }
                }
                _ => blocks.push((*c, vec![it.id.0])),
            }
        }
    }
    let all = items(&seq);
    let ways: Vec<usize> = seq.items.iter().map(|i| i.way).collect();
    let c = cost(&all, &ways, &seq.ends);
    let pt = |p: Option<Point>| p.map_or(Value::Null, |p| json!({"x": p.x, "y": p.y}));
    Ok(json!({
        "objects": objects,
        "colorBlocks": blocks.into_iter().map(|(rgb, ids)| json!({"rgb": hex(rgb), "objects": ids})).collect::<Vec<_>>(),
        "colorChanges": c.changes,
        "trims": c.trims,
        "travelMm": (c.travel / UNITS_PER_MM * 10.0).round() / 10.0,
        "ordered": !d.stitch.order.is_empty(),
        "start": pt(d.stitch.start_point()),
        "end": pt(d.stitch.end_point()),
    }))
}

/// Store `order` as the sewing order, as one undo step `label` (none when it is unchanged).
fn store_order(s: &mut Session, label: &str, order: Vec<NodeId>) -> Result<Value> {
    let ids: Vec<u64> = order.iter().map(|i| i.0).collect();
    if s.doc()?.doc.stitch.order != order {
        s.edit(label, |d, _| {
            d.stitch.order = order;
            Ok(())
        })?;
    }
    Ok(json!({ "order": ids }))
}

/// An id parameter: a whole number.
fn id_of(v: &Value, cmd: &str, what: &str) -> Result<NodeId> {
    v.as_u64().map(NodeId).ok_or_else(|| bad(cmd, format!("{what} must be an object id, not {v}")))
}

fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_of(p.get("id").ok_or_else(|| bad(MOVE, "id is required: the object to move"))?, MOVE, "id")?;
    let to = p.get("to").ok_or_else(|| bad(MOVE, "to is required: its new index in the sewing order"))?;
    let to = to.as_u64().ok_or_else(|| bad(MOVE, format!("to must be a whole number 0 or more, not {to}")))?;
    let seq = sequenced(s, &json!({}), MOVE)?;
    let mut order: Vec<NodeId> = seq.items.iter().map(|i| i.id).collect();
    let at = order.iter().position(|i| *i == id).ok_or_else(|| bad(MOVE, format!("object {} is not sewn", id.0)))?;
    order.remove(at);
    let to = usize::try_from(to).unwrap_or(usize::MAX).min(order.len());
    order.insert(to, id);
    store_order(s, "Move in Stitch Sequence", order)
}

fn set_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = p.get("ids").and_then(Value::as_array).ok_or_else(|| bad(SET, "ids must be an array of object ids ([]: document order)"))?;
    let seq = sequenced(s, &json!({}), SET)?;
    let mut order: Vec<NodeId> = Vec::with_capacity(ids.len());
    for v in ids {
        let id = id_of(v, SET, "each of ids")?;
        if !seq.natural.contains(&id) {
            return Err(bad(SET, format!("object {} is not sewn", id.0)));
        }
        if order.contains(&id) {
            return Err(bad(SET, format!("object {} is listed twice", id.0)));
        }
        order.push(id);
    }
    store_order(s, "Set Stitch Sequence", order)
}

fn sort_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let seq = sequenced(s, p, SORT)?;
    let all = items(&seq);
    let perm = colour_sort(&all);
    let identity: Vec<usize> = (0..all.len()).collect();
    let before = colour_changes(&all, &identity);
    let after = colour_changes(&all, &perm);
    let changed = perm != identity;
    let order: Vec<NodeId> = perm.iter().filter_map(|&i| seq.items.get(i)).map(|i| i.id).collect();
    let mut r =
        if changed { store_order(s, "Sort Sequence by Color", order)? } else { json!({"order": order.iter().map(|i| i.0).collect::<Vec<_>>()}) };
    r["changed"] = json!(changed);
    r["colorChanges"] = json!({"before": before, "after": after});
    Ok(r)
}

/// A point parameter: `{x, y}` or `[x, y]`, finite and on the canvas.
fn point_of(v: &Value, cmd: &str, key: &str) -> Result<Point> {
    let (x, y) = match v {
        Value::Array(a) if a.len() == 2 => (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)),
        Value::Object(o) => (o.get("x").and_then(Value::as_f64), o.get("y").and_then(Value::as_f64)),
        _ => (None, None),
    };
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x.abs() <= MAX_COORD && y.abs() <= MAX_COORD => Ok(Point::new(x, y)),
        _ => Err(bad(cmd, format!("{key} must be a point {{x, y}} on the canvas (or null), not {v}"))),
    }
}

/// What `stitch.entryExit.set` does to one point: leave it, unset it, set a fraction or the
/// fraction nearest a point.
#[derive(Clone, Copy)]
enum PointEdit {
    Keep,
    Unset,
    At(f64),
    Near(Pt),
}

fn point_edit(p: &Value, key: &str) -> Result<PointEdit> {
    match p.get(key) {
        None => Ok(PointEdit::Keep),
        Some(Value::Null) => Ok(PointEdit::Unset),
        Some(Value::Number(n)) => match n.as_f64().filter(|t| t.is_finite() && (0.0..=1.0).contains(t)) {
            Some(t) => Ok(PointEdit::At(t)),
            None => Err(bad(ENTRY_EXIT, format!("{key} as a fraction along the outline must be 0–1, not {n}"))),
        },
        Some(v) => {
            let q = point_of(v, ENTRY_EXIT, key)?;
            Pt::from_pt(q.x, q.y).map(PointEdit::Near).ok_or_else(|| bad(ENTRY_EXIT, format!("{key} lies too far from the origin")))
        }
    }
}

/// The paths and compound paths `p` (or the selection) names: a compound path's member stands for
/// its compound path; other kinds are skipped. An id naming no object is an error.
fn shape_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let mut out: Vec<NodeId> = vec![];
    for id in targets(s, p)? {
        let n = d.node(id).ok_or_else(|| bad(ENTRY_EXIT, format!("no object {}", id.0)))?;
        let owner = match d.parent_of(id).and_then(|pid| d.node(pid)) {
            Some(parent) if matches!(parent.kind, NodeKind::Compound { .. }) => parent,
            _ => n,
        };
        if matches!(owner.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) && !out.contains(&owner.id) {
            out.push(owner.id);
        }
    }
    if out.is_empty() {
        return Err(bad(ENTRY_EXIT, "select paths or compound paths"));
    }
    Ok(out)
}

fn entry_exit(s: &mut Session, p: &Value) -> Result<Value> {
    let clear = match p.get("clear") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(v) => return Err(bad(ENTRY_EXIT, format!("clear must be true or false, not {v}"))),
    };
    let entry = point_edit(p, "entry")?;
    let exit = point_edit(p, "exit")?;
    if !clear && matches!((entry, exit), (PointEdit::Keep, PointEdit::Keep)) {
        return Err(bad(ENTRY_EXIT, "give entry, exit or clear"));
    }
    let ids = shape_targets(s, p)?;
    // Each object's new properties, worked out before the edit so nothing changes on an error.
    let d = &s.doc()?.doc;
    let mut updates = vec![];
    for id in ids {
        let Some(n) = d.node(id) else { continue };
        let before = n.stitch_props().copied().unwrap_or_default();
        let mut after = before;
        if clear {
            (after.entry, after.exit, after.join_entry, after.join_exit) = (None, None, None, None);
        }
        for (edit, slot) in [(entry, &mut after.entry), (exit, &mut after.exit)] {
            match edit {
                PointEdit::Keep => {}
                PointEdit::Unset => *slot = None,
                PointEdit::At(t) => *slot = Some(t),
                PointEdit::Near(q) => *slot = outline_fraction(n, q),
            }
        }
        if after != before {
            updates.push((id, after));
        }
    }
    if updates.is_empty() {
        return Ok(json!({ "changed": [] }));
    }
    let changed: Vec<u64> = updates.iter().map(|(id, _)| id.0).collect();
    s.edit("Set Entry and Exit Points", |d, _| {
        for (id, props) in updates {
            if let Some(n) = d.node_mut(id) {
                n.set_stitch_props(props);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "changed": changed }))
}

fn join_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let seq = sequenced(s, p, JOIN)?;
    let all = items(&seq);
    let current: Vec<usize> = seq.items.iter().map(|i| i.way).collect();
    let ways = closest_join(&all, &current, &seq.ends);
    let before = cost(&all, &current, &seq.ends);
    let after = cost(&all, &ways, &seq.ends);
    let d = &s.doc()?.doc;
    let mut updates = vec![];
    for (it, w) in seq.items.iter().zip(&ways) {
        // Pinned objects and an opened file's art have one way: nothing to choose.
        if it.item.ways.len() < 2 {
            continue;
        }
        let (Some(way), Some(n)) = (it.item.ways.get(*w), d.node(it.id)) else { continue };
        let before = n.stitch_props().copied().unwrap_or_default();
        let mut props = before;
        props.join_entry = outline_fraction(n, way.entry);
        props.join_exit = outline_fraction(n, way.exit);
        if props != before {
            updates.push((it.id, props));
        }
    }
    let changed: Vec<u64> = updates.iter().map(|(id, _)| id.0).collect();
    if !updates.is_empty() {
        s.edit("Closest Join", |d, _| {
            for (id, props) in updates {
                if let Some(n) = d.node_mut(id) {
                    n.set_stitch_props(props);
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({ "changed": changed, "before": cost_json(&before), "after": cost_json(&after) }))
}

fn start_end(s: &mut Session, p: &Value) -> Result<Value> {
    let read = |key: &str| -> Result<Option<Option<Point>>> {
        match p.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(v) => Ok(Some(Some(point_of(v, START_END, key)?))),
        }
    };
    let (start, end) = (read("start")?, read("end")?);
    if start.is_none() && end.is_none() {
        return Err(bad(START_END, "give start, end or both ({x, y}, or null to unset)"));
    }
    let now = s.doc()?.doc.stitch.clone();
    let mut next = now.clone();
    if let Some(v) = start {
        next.start = v;
    }
    if let Some(v) = end {
        next.end = v;
    }
    if next != now {
        let value = next.clone();
        s.edit("Set Design Start and End", |d, _| {
            d.stitch = value;
            Ok(())
        })?;
    }
    let pt = |p: Option<Point>| p.map_or(Value::Null, |p| json!({"x": p.x, "y": p.y}));
    Ok(json!({ "start": pt(next.start), "end": pt(next.end) }))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::Session;

    /// A document of stroked lines: `(x, colour)` each a vertical line 0–100 at `x`, drawn
    /// downwards, bottom to top in the stacking order.
    fn session(lines: &[(f64, &str)]) -> (Session, Vec<u64>) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 2000, "height": 2000})).unwrap();
        s.execute("paint.setFill", &json!({"none": true})).unwrap();
        let mut ids = vec![];
        for (x, c) in lines {
            // Nothing selected, so the colour is the next object's and not the last one's.
            s.execute("select.none", &json!({})).unwrap();
            s.execute("paint.setStroke", &json!({"color": c})).unwrap();
            let (y0, y1) = (0.0, 100.0);
            let id = s.execute("shape.line", &json!({"x1": x, "y1": y0, "x2": x, "y2": y1})).unwrap()["id"].as_u64().unwrap();
            ids.push(id);
        }
        (s, ids)
    }

    /// A point within a stitch unit (0.1 mm) of (`x`, `y`).
    fn near(v: &Value, x: f64, y: f64) -> bool {
        (v["x"].as_f64().unwrap() - x).hypot(v["y"].as_f64().unwrap() - y) < 0.3
    }

    fn order(s: &mut Session) -> Vec<u64> {
        let l = s.execute("stitch.sequence.list", &json!({})).unwrap();
        l["objects"].as_array().unwrap().iter().map(|o| o["id"].as_u64().unwrap()).collect()
    }

    #[test]
    fn move_set_and_sort_change_the_order_one_undo_step_each() {
        let (mut s, ids) = session(&[(0.0, "#ff0000"), (50.0, "#0000ff"), (100.0, "#ff0000")]);
        assert_eq!(order(&mut s), ids);
        let l = s.execute("stitch.sequence.list", &json!({})).unwrap();
        assert_eq!((l["colorChanges"].as_u64(), l["ordered"].as_bool()), (Some(2), Some(false)), "{l}");
        assert_eq!(l["colorBlocks"].as_array().unwrap().len(), 3);
        s.execute("stitch.sequence.move", &json!({"id": ids[2], "to": 0})).unwrap();
        assert_eq!(order(&mut s), vec![ids[2], ids[0], ids[1]]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(order(&mut s), ids);
        let r = s.execute("stitch.sequence.sortByColor", &json!({})).unwrap();
        assert_eq!(r["colorChanges"], json!({"before": 2, "after": 1}));
        assert_eq!(order(&mut s), vec![ids[0], ids[2], ids[1]]);
        s.execute("stitch.sequence.set", &json!({"ids": [ids[1]]})).unwrap();
        assert_eq!(order(&mut s), vec![ids[1], ids[0], ids[2]]);
        s.execute("stitch.sequence.set", &json!({"ids": []})).unwrap();
        assert_eq!(order(&mut s), ids);
        // The stitch file sews in the order: sorted, one colour change.
        s.execute("stitch.sequence.sortByColor", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.info", &json!({})).unwrap()["colorChanges"], json!(1));
    }

    #[test]
    fn bad_params_are_refused() {
        let (mut s, ids) = session(&[(0.0, "#ff0000"), (50.0, "#0000ff")]);
        for (c, p) in [
            ("stitch.sequence.move", json!({"id": ids[0]})),
            ("stitch.sequence.move", json!({"id": 99999, "to": 0})),
            ("stitch.sequence.move", json!({"id": ids[0], "to": -1})),
            ("stitch.sequence.set", json!({"ids": [ids[0], ids[0]]})),
            ("stitch.sequence.set", json!({"ids": [99999]})),
            ("stitch.sequence.set", json!({})),
            ("stitch.entryExit.set", json!({"ids": [ids[0]]})),
            ("stitch.entryExit.set", json!({"ids": [ids[0]], "entry": 2})),
            ("stitch.entryExit.set", json!({"ids": [ids[0]], "entry": "start"})),
            ("stitch.designStartEnd.set", json!({})),
            ("stitch.designStartEnd.set", json!({"start": {"x": "a"}})),
            ("stitch.join.closest", json!({"options": 3})),
        ] {
            assert!(s.execute(c, &p).is_err(), "{c} {p}");
        }
        assert_eq!(order(&mut s), ids);
    }

    #[test]
    fn entry_points_and_closest_join_turn_the_lines() {
        // Lines all drawn downwards: in document order each starts far from where the one before
        // it ended.
        let (mut s, ids) = session(&[(0.0, "#ff0000"), (20.0, "#ff0000"), (40.0, "#ff0000")]);
        let info = |s: &mut Session| s.execute("stitch.info", &json!({})).unwrap();
        let before = info(&mut s);
        let r = s.execute("stitch.join.closest", &json!({})).unwrap();
        assert!(r["after"]["travelMm"].as_f64() < r["before"]["travelMm"].as_f64(), "{r}");
        assert_eq!(r["changed"].as_array().unwrap().len(), 3);
        let after = info(&mut s);
        assert!(after["travelMm"].as_f64() < before["travelMm"].as_f64(), "{before} → {after}");
        assert!(after["trims"].as_u64() <= before["trims"].as_u64());
        // A second closest join changes nothing.
        assert_eq!(s.execute("stitch.join.closest", &json!({})).unwrap()["changed"], json!([]));
        // The user's entry point wins over closest join's, and moves with the outline.
        s.execute("stitch.entryExit.set", &json!({"ids": [ids[0]], "entry": {"x": 0, "y": 100}})).unwrap();
        let l = s.execute("stitch.sequence.list", &json!({})).unwrap();
        let first = &l["objects"][0];
        assert_eq!((first["pinned"].as_bool(), first["own"]["entry"].as_f64()), (Some(true), Some(1.0)), "{first}");
        assert!(near(&first["entry"], 0.0, 100.0), "{first}");
        s.execute("stitch.entryExit.set", &json!({"ids": [ids[0]], "clear": true})).unwrap();
        assert_eq!(s.execute("stitch.sequence.list", &json!({})).unwrap()["objects"][0]["own"], json!({}));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.sequence.list", &json!({})).unwrap()["objects"][0]["pinned"], json!(true));
    }

    #[test]
    fn the_design_start_turns_the_first_object_and_is_the_file_origin() {
        let (mut s, _) = session(&[(0.0, "#ff0000")]);
        let r = s.execute("stitch.designStartEnd.set", &json!({"start": [0, 100]})).unwrap();
        assert_eq!(r, json!({"start": {"x": 0.0, "y": 100.0}, "end": Value::Null}));
        let l = s.execute("stitch.sequence.list", &json!({})).unwrap();
        assert!(near(&l["objects"][0]["entry"], 0.0, 100.0), "{l}");
        assert_eq!(l["start"], json!({"x": 0.0, "y": 100.0}));
        s.execute("stitch.designStartEnd.set", &json!({"start": null, "end": {"x": 1, "y": 2}})).unwrap();
        let l = s.execute("stitch.sequence.list", &json!({})).unwrap();
        assert_eq!((l["start"].clone(), l["end"].clone()), (Value::Null, json!({"x": 1.0, "y": 2.0})));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.sequence.list", &json!({})).unwrap()["end"], Value::Null);
    }
}
