//! Embroidery › Stitch Properties: the stitch type and parameters of objects
//! ([`vectorcraft_doc::StitchProps`]): `stitch.setType`, `stitch.setParams` and `stitch.clear`
//! change them (one undo step each), `stitch.props` reads what each object sews with. Groups and
//! layers pass theirs down; an export makes the stitches from the outlines with them
//! ([`vectorcraft_stitch::from_document`]).

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, LetterJoin, LetterTrims, Node, NodeId, NodeKind, StitchParam, StitchProps, StitchType, Underlay};
use vectorcraft_stitch::{FromDocOptions, effective_number, inherited_props, sewn_as};

use super::super::fileio::stitchfile;
use super::super::{CommandSpec, Result, Session, bad, cmd, has_doc, targets};

const SET_TYPE: &str = "stitch.setType";
const SET_PARAMS: &str = "stitch.setParams";
const CLEAR: &str = "stitch.clear";
const PROPS: &str = "stitch.props";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stitch.setType",
            "Set Stitch Type",
            [],
            None,
            "{type: auto (the document's stitch options decide: a fill sewn as tatami, then a stroke as running stitch or satin) | none (not sewn) | run (running stitch along the outline) | satin (a satin column centred on the path) | tatami (tatami fill of the region the outline encloses) | manual (one needle point per anchor) | lettering (type sewn as satin lettering, as automatic type is; on a path it is automatic), ids?|id? (default: the selection)} the stitch type of paths, compound paths, type and groups (a group's or layer's passes to the objects in it that leave theirs automatic; a compound path's member stands for the compound path); one type sews the object once, in its stroke colour (run, satin, manual) or fill colour (tatami), the other paint's colour when it has only that; objects of other kinds are skipped; not used on the layer an opened stitch file made; one undo step → {changed: [id…]}",
            has_doc,
            set_type
        ),
        cmd!(
            "stitch.setParams",
            "Set Stitch Parameters",
            [],
            None,
            "{params: {fillAngle?: degrees -360–360 (tatami row direction, counter-clockwise), fillSpacing?: mm 0.2–5 (between tatami rows), fillStitchLength?: mm 1–8 (along a row), stagger?: 0–1 (row offset, a fraction of the stitch length), satinSpacing?: mm 0.2–5 (peak to peak), satinWidth?: mm 0.5–20 (rail to rail; unset: the stroke weight), runLength?: mm 0.5–12 (running stitch length; lettering's runs 1.5), underlay?: auto | none | center | edge | zigzag (under two-rail satin columns and satin lettering; auto, lettering's default, chooses by the column's width as drawn: a centre run under 3 mm, an edge walk from 3 mm, an edge walk and a zigzag from 6 mm; true is auto and false none; not yet under fills), compensation?: mm 0–2 (pull compensation each side, for two-rail satin columns (default 0) and lettering (default 0.2)), minColumn?: mm 0.3–3 (lettering: narrower strokes are running stitch; default 1), runUnder?: mm 0–20 (lettering: letters shorter than this are running stitch; default 5), letterTrims?: word (default) | letter | none (lettering: where the thread is trimmed between letters), letterJoin?: closest (default) | bottom (lettering: each letter starts at its point nearest the needle, or near its bottom left, joined along the baseline), bean?: bool (lettering: running-stitch letters sewn as a bean run, each stitch three times; default false)} (null unsets one: it is inherited again, from the groups holding the object, else from the document's stitch options), ids?|id? (default: the selection)} the stitch parameters of paths, compound paths and groups, over the document's stitch options (the export's fill, stroke and stitchLength); what is not given stays as it was; one undo step → {changed: [id…]}",
            has_doc,
            set_params
        ),
        cmd!(
            "stitch.clear",
            "Clear Stitch Properties",
            [],
            None,
            "{ids?|id? (default: the selection)} remove the stitch type and parameters (and the entry and exit points) of paths, compound paths and groups: they are automatic again and inherit everything; one undo step → {changed: [id…] (those that had any)}",
            has_doc,
            clear
        ),
        cmd!(
            query "stitch.props",
            "Stitch Properties",
            [],
            None,
            "{ids?|id? (default: the selection), options?: {…the stitch options of stitch.info (stitchLength, mode, fill, stroke…): the document's defaults; default: the export's}} what each path, compound path and group sews with → {objects: [{id, kind: path|compound|group|layer, own: {type?, fillAngle?…} (what it sets itself), type: the type it sews with after inheritance (auto: the options decide), sews: [tatami|run|satin|manual…] (what an export sews, in order; for a group or layer, what the objects in it sew, each type once; [] when nothing is sewn), params: {fillAngle, fillSpacing, fillStitchLength, stagger, satinSpacing, satinWidth (null: the stroke weight), runLength, underlay (auto|none|center|edge|zigzag; unset reads none, lettering's default auto), compensation (null: none), letterTrims, letterJoin, bean} (after inheritance and the options), opened: true when on the layer an opened stitch file made (stitch properties are not used there), overlapMm: the allowance stitch.overlap.remove left on it (null: sewn whole), routeMm: the longest travel stitch.route lets it be entered by (null: a trim)}]}; objects of other kinds are left out",
            has_doc,
            props
        ),
    ]
}

/// The objects whose stitch properties `p` (or the selection) names: a compound path's member
/// stands for its compound path; objects of other kinds than paths, compound paths, groups and
/// layers are left out. An id naming no object is an error.
fn stitch_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let mut out: Vec<NodeId> = vec![];
    for id in targets(s, p)? {
        let n = d.node(id).ok_or_else(|| bad(cmd, format!("no object {}", id.0)))?;
        let owner = match d.parent_of(id).and_then(|pid| d.node(pid)) {
            Some(parent) if matches!(parent.kind, NodeKind::Compound { .. }) => parent,
            _ => n,
        };
        let takes = matches!(
            owner.kind,
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Text(_)
        );
        if takes && !out.contains(&owner.id) {
            out.push(owner.id);
        }
    }
    if out.is_empty() {
        return Err(bad(cmd, "select paths, compound paths or groups"));
    }
    Ok(out)
}

/// Change the stitch properties of the objects `p` names with `f`, as one undo step `label` →
/// `{changed}`: the objects whose properties changed.
fn change(s: &mut Session, p: &Value, cmd: &str, label: &str, f: impl Fn(&mut StitchProps)) -> Result<Value> {
    let ids = stitch_targets(s, p, cmd)?;
    let changed = s.edit(label, |d, _| {
        let mut changed = vec![];
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let before = n.stitch_props().copied().unwrap_or_default();
            let mut after = before;
            f(&mut after);
            if after != before {
                n.set_stitch_props(after);
                changed.push(id.0);
            }
        }
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

fn set_type(s: &mut Session, p: &Value) -> Result<Value> {
    let given = p.get("type").ok_or_else(|| bad(SET_TYPE, "type is required: auto, none, run, satin, tatami, manual or lettering"))?;
    let kind = given
        .as_str()
        .and_then(StitchType::parse)
        .ok_or_else(|| bad(SET_TYPE, format!("type {given}: auto, none, run, satin, tatami, manual or lettering")))?;
    change(s, p, SET_TYPE, "Set Stitch Type", |props| props.kind = kind)
}

/// One change `stitch.setParams` makes: a number set or unset, the underlay, or lettering's trims.
enum Edit {
    Number(StitchParam, Option<f64>),
    Underlay(Option<Underlay>),
    LetterTrims(Option<LetterTrims>),
    LetterJoin(Option<LetterJoin>),
    Bean(Option<bool>),
}

/// The changes `params` asks for, checked: an unknown key, a value of the wrong kind or out of
/// range is an error and nothing changes.
fn param_edits(params: &Map<String, Value>) -> Result<Vec<Edit>> {
    let keys = || StitchParam::ALL.iter().map(|p| p.key()).chain(["underlay", "letterTrims", "letterJoin", "bean"]).collect::<Vec<_>>().join(", ");
    let mut edits = vec![];
    for (k, v) in params {
        if k == "letterTrims" {
            let trims = match v {
                Value::Null => None,
                v => Some(
                    v.as_str()
                        .and_then(LetterTrims::parse)
                        .ok_or_else(|| bad(SET_PARAMS, format!("params.letterTrims must be word, letter, none or null, not {v}")))?,
                ),
            };
            edits.push(Edit::LetterTrims(trims));
            continue;
        }
        if k == "underlay" {
            let kind = match v {
                Value::Null => None,
                Value::Bool(true) => Some(Underlay::Auto),
                Value::Bool(false) => Some(Underlay::None),
                v => Some(v.as_str().and_then(Underlay::parse).ok_or_else(|| {
                    bad(SET_PARAMS, format!("params.underlay must be auto, none, center, edge, zigzag, true, false or null, not {v}"))
                })?),
            };
            edits.push(Edit::Underlay(kind));
            continue;
        }
        if k == "letterJoin" {
            let join = match v {
                Value::Null => None,
                v => Some(
                    v.as_str()
                        .and_then(LetterJoin::parse)
                        .ok_or_else(|| bad(SET_PARAMS, format!("params.letterJoin must be closest, bottom or null, not {v}")))?,
                ),
            };
            edits.push(Edit::LetterJoin(join));
            continue;
        }
        if k == "bean" {
            let on = match v {
                Value::Null => None,
                Value::Bool(b) => Some(*b),
                _ => return Err(bad(SET_PARAMS, format!("params.bean must be true, false or null, not {v}"))),
            };
            edits.push(Edit::Bean(on));
            continue;
        }
        let param = StitchParam::parse(k).ok_or_else(|| bad(SET_PARAMS, format!("unknown parameter `params.{k}`; params takes: {}", keys())))?;
        let value = match v {
            Value::Null => None,
            v => {
                let (lo, hi) = param.range();
                let unit = if param.unit().is_empty() { String::new() } else { format!(" {}", param.unit()) };
                let n = v.as_f64().ok_or_else(|| bad(SET_PARAMS, format!("params.{k} must be a number or null, not {v}")))?;
                Some(param.checked(n).ok_or_else(|| bad(SET_PARAMS, format!("params.{k} must be {lo}–{hi}{unit}, not {n}")))?)
            }
        };
        edits.push(Edit::Number(param, value));
    }
    Ok(edits)
}

fn set_params(s: &mut Session, p: &Value) -> Result<Value> {
    let params =
        p.get("params").and_then(Value::as_object).ok_or_else(|| bad(SET_PARAMS, "params must be an object, such as {\"fillSpacing\": 0.5}"))?;
    let edits = param_edits(params)?;
    change(s, p, SET_PARAMS, "Set Stitch Parameters", |props| {
        for e in &edits {
            match *e {
                Edit::Number(param, v) => props.set_number(param, v),
                Edit::Underlay(v) => props.underlay = v,
                Edit::LetterTrims(v) => props.letter_trims = v,
                Edit::LetterJoin(v) => props.letter_join = v,
                Edit::Bean(v) => props.bean = v,
            }
        }
    })
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    change(s, p, CLEAR, "Clear Stitch Properties", |props| *props = StitchProps::default())
}

/// The kind name `stitch.props` gives an object.
fn kind_name(k: &NodeKind) -> &'static str {
    match k {
        NodeKind::Compound { .. } => "compound",
        NodeKind::Group { .. } => "group",
        NodeKind::Layer { .. } => "layer",
        NodeKind::Text(_) => "text",
        _ => "path",
    }
}

/// `props`' fields as JSON: those set, or (`all`) every field, unset ones as null.
fn props_json(props: &StitchProps, number: impl Fn(StitchParam) -> Option<f64>, all: bool) -> Map<String, Value> {
    let mut m = Map::new();
    if all || props.kind != StitchType::Auto {
        m.insert("type".into(), json!(props.kind.id()));
    }
    for param in StitchParam::ALL {
        let v = number(param);
        if all || v.is_some() {
            m.insert(param.key().into(), json!(v));
        }
    }
    if all || props.underlay.is_some() {
        m.insert("underlay".into(), json!(props.underlay.unwrap_or(Underlay::None).id()));
    }
    if all || props.letter_trims.is_some() {
        m.insert("letterTrims".into(), json!(props.letter_trims.unwrap_or_default().id()));
    }
    if all || props.letter_join.is_some() {
        m.insert("letterJoin".into(), json!(props.letter_join.unwrap_or_default().id()));
    }
    if all || props.bean.is_some() {
        m.insert("bean".into(), json!(props.bean.unwrap_or(false)));
    }
    m
}

/// What the paths and compound paths in group or layer `g` sew, each type once, in the order
/// they are first sewn: empty when nothing in it is sewn.
fn contents_sewn_as(d: &Document, g: &Node, o: &FromDocOptions) -> Vec<StitchType> {
    let mut out: Vec<StitchType> = vec![];
    g.walk(&mut |n: &Node| {
        // A compound path answers for its paths; and once every type is found, nothing can be added.
        let in_compound = d.parent_of(n.id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
        if in_compound || out.len() >= StitchType::ALL.len() {
            return;
        }
        let Some((props, opened)) = inherited_props(d, n.id) else { return };
        for t in sewn_as(n, &props, opened, o) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    });
    out
}

fn props(s: &mut Session, p: &Value) -> Result<Value> {
    let options = match p.get("options") {
        None | Some(Value::Null) => json!({}),
        Some(o) if o.is_object() => o.clone(),
        Some(o) => return Err(bad(PROPS, format!("options must be an object of stitch options, not {o}"))),
    };
    let o = stitchfile::from_doc_options(PROPS, &options)?;
    let ids = stitch_targets(s, p, PROPS)?;
    let d: &Document = &s.doc()?.doc;
    let mut objects = vec![];
    for id in ids {
        let (Some(n), Some((inherited, opened))) = (d.node(id), inherited_props(d, id)) else { continue };
        let own = n.stitch_props().copied().unwrap_or_default();
        let sews: Vec<&str> = match n.kind {
            // Converted lettering sews as lettering, its pieces as they are set.
            NodeKind::Group { .. } if own.kind == StitchType::Lettering && !opened => {
                let mut v = contents_sewn_as(d, n, &o);
                v.retain(|t| *t != StitchType::Lettering);
                v.insert(0, StitchType::Lettering);
                v
            }
            NodeKind::Group { .. } | NodeKind::Layer { .. } => contents_sewn_as(d, n, &o),
            _ => sewn_as(n, &inherited, opened, &o),
        }
        .into_iter()
        .map(StitchType::id)
        .collect();
        objects.push(json!({
            "id": id.0,
            "kind": kind_name(&n.kind),
            "own": props_json(&own, |param| own.number(param), false),
            "type": inherited.kind.id(),
            "sews": sews,
            "params": props_json(&inherited, |param| effective_number(&inherited, param, &o), true),
            "opened": opened,
            "overlapMm": own.overlap_mm(),
            "routeMm": own.route_mm(),
        }));
    }
    Ok(json!({ "objects": objects }))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
        s
    }

    fn rect(s: &mut Session, x: f64, w: f64) -> u64 {
        s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": w, "height": w})).unwrap()["id"].as_u64().unwrap()
    }

    fn props_of(s: &mut Session, id: u64) -> Value {
        s.execute("stitch.props", &json!({"ids": [id]})).unwrap()["objects"][0].clone()
    }

    #[test]
    fn set_type_and_params_are_one_undo_step_each_and_read_back() {
        let mut s = session();
        let id = rect(&mut s, 0.0, 100.0);
        let auto = props_of(&mut s, id);
        assert_eq!((auto["type"].as_str(), auto["sews"].clone()), (Some("auto"), json!(["tatami", "run"])), "{auto}");
        assert_eq!(auto["own"], json!({}));
        assert_eq!(auto["params"]["fillSpacing"], json!(0.4));
        assert_eq!(auto["params"]["satinWidth"], Value::Null);
        let r = s.execute("stitch.setType", &json!({"type": "satin"})).unwrap();
        assert_eq!(r["changed"], json!([id]));
        s.execute("stitch.setParams", &json!({"params": {"satinWidth": 4, "underlay": true}})).unwrap();
        let p = props_of(&mut s, id);
        assert_eq!((p["type"].as_str(), p["sews"].clone()), (Some("satin"), json!(["satin"])));
        assert_eq!(p["own"], json!({"type": "satin", "satinWidth": 4.0, "underlay": "auto"}));
        assert_eq!(p["params"]["satinWidth"], json!(4.0));
        // Each was one undo step.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(props_of(&mut s, id)["own"], json!({"type": "satin"}));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(props_of(&mut s, id)["own"], json!({}));
        s.execute("edit.redo", &json!({})).unwrap();
        // null unsets one; clear removes them all.
        s.execute("stitch.setParams", &json!({"params": {"fillSpacing": 0.5}})).unwrap();
        s.execute("stitch.setParams", &json!({"params": {"fillSpacing": null}})).unwrap();
        assert_eq!(props_of(&mut s, id)["own"], json!({"type": "satin"}));
        let r = s.execute("stitch.clear", &json!({"ids": [id]})).unwrap();
        assert_eq!(r["changed"], json!([id]));
        assert_eq!(props_of(&mut s, id)["own"], json!({}));
        assert_eq!(s.execute("stitch.clear", &json!({"ids": [id]})).unwrap()["changed"], json!([]), "nothing left to clear");
    }

    #[test]
    fn bad_params_are_refused_and_change_nothing() {
        let mut s = session();
        let id = rect(&mut s, 0.0, 100.0);
        for p in [json!({}), json!({"type": "zigzag"}), json!({"type": 3})] {
            assert!(s.execute("stitch.setType", &p).is_err(), "{p}");
        }
        for p in [
            json!({}),
            json!({"params": 5}),
            json!({"params": {"fillSpacing": 0.1}}),
            json!({"params": {"fillSpacing": "wide"}}),
            json!({"params": {"density": 2}}),
            json!({"params": {"underlay": 1}}),
            json!({"params": {"runLength": 3, "satinWidth": 99}}),
        ] {
            assert!(s.execute("stitch.setParams", &p).is_err(), "{p}");
        }
        assert_eq!(props_of(&mut s, id)["own"], json!({}));
        // Nothing selected, an unknown id, and an object of another kind.
        s.execute("select.none", &json!({})).unwrap();
        assert!(s.execute("stitch.setType", &json!({"type": "run"})).is_err());
        assert!(s.execute("stitch.setType", &json!({"type": "run", "ids": [99999]})).is_err());
        assert!(s.execute("stitch.props", &json!({"options": {"stitchLength": 99}, "ids": [id]})).is_err());
    }

    #[test]
    fn groups_pass_their_type_down_and_options_are_the_defaults() {
        let mut s = session();
        let a = rect(&mut s, 0.0, 50.0);
        let b = rect(&mut s, 100.0, 50.0);
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("stitch.setType", &json!({"type": "run", "ids": [g]})).unwrap();
        s.execute("stitch.setParams", &json!({"params": {"runLength": 3.5}, "ids": [g]})).unwrap();
        s.execute("stitch.setParams", &json!({"params": {"runLength": 2}, "ids": [b]})).unwrap();
        let pa = props_of(&mut s, a);
        assert_eq!((pa["type"].as_str(), pa["sews"].clone(), pa["params"]["runLength"].as_f64()), (Some("run"), json!(["run"]), Some(3.5)));
        assert_eq!(props_of(&mut s, b)["params"]["runLength"].as_f64(), Some(2.0));
        let group = props_of(&mut s, g);
        // A group sews what the objects in it sew: here both rectangles, as running stitch.
        assert_eq!((group["kind"].as_str(), group["sews"].clone()), (Some("group"), json!(["run"])));
        // Left automatic, the filled and stroked rectangles are sewn as tatami, then running stitch.
        s.execute("stitch.setType", &json!({"type": "auto", "ids": [g]})).unwrap();
        assert_eq!(props_of(&mut s, g)["sews"], json!(["tatami", "run"]));
        // Only a group in which nothing is sewn sews nothing.
        s.execute("stitch.setType", &json!({"type": "none", "ids": [g]})).unwrap();
        assert_eq!(props_of(&mut s, g)["sews"], json!([]));
        s.execute("stitch.setType", &json!({"type": "run", "ids": [g]})).unwrap();
        // The options stand for the document's defaults where nothing is set.
        let r = s.execute("stitch.props", &json!({"ids": [a], "options": {"fill": {"spacing": 0.8}}})).unwrap();
        assert_eq!(r["objects"][0]["params"]["fillSpacing"], json!(0.8));
        // Stitch Info sews the group's paths as running stitch only: one thread, red.
        let info = s.execute("stitch.info", &json!({})).unwrap();
        assert_eq!(info["threads"].as_array().map(Vec::len), Some(1), "{info}");
        assert_eq!(info["threads"][0]["rgb"], json!("#ff0000"));
    }

    /// An opened stitch file is a layer of groups of runs: a click selects a group, and the group
    /// says what its runs are sewn as, not that nothing is sewn (STI-33).
    #[test]
    fn a_group_of_an_opened_stitch_file_is_sewn_as_its_needle_points() {
        let mut s = session();
        rect(&mut s, 0.0, 50.0);
        let file = s.execute("document.export", &json!({"format": "dst"})).unwrap();
        s.execute("document.open", &json!({"name": "a.dst", "dataBase64": file["dataBase64"]})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        let objects = s.execute("stitch.props", &json!({})).unwrap()["objects"].clone();
        let objects = objects.as_array().unwrap();
        assert!(!objects.is_empty());
        for o in objects {
            assert_eq!(o["opened"], true, "{o}");
            assert_eq!(o["sews"], json!(["manual"]), "{o}");
        }
        assert!(objects.iter().any(|o| o["kind"] == "group"), "the runs are grouped by colour: {objects:?}");
    }

    /// Moving or scaling an object sews it again from its outline, at the same density.
    #[test]
    fn scaling_an_object_makes_its_stitches_again() {
        let mut s = session();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        let id = rect(&mut s, 0.0, 60.0);
        s.execute("stitch.setParams", &json!({"params": {"fillSpacing": 0.5}})).unwrap();
        let before = s.execute("stitch.info", &json!({})).unwrap();
        s.execute("object.scale", &json!({"ids": [id], "sx": 200, "sy": 200})).unwrap();
        let after = s.execute("stitch.info", &json!({})).unwrap();
        let ratio = after["stitches"].as_f64().unwrap() / before["stitches"].as_f64().unwrap();
        assert!((3.5..=4.5).contains(&ratio), "{before} → {after}: × {ratio}");
        let w = |v: &Value| v["widthMm"].as_f64().unwrap();
        assert!((w(&after) / w(&before) - 2.0).abs() < 0.05);
        assert_eq!(props_of(&mut s, id)["own"], json!({"fillSpacing": 0.5}), "the properties stay with the object");
    }

    #[test]
    fn a_compound_members_properties_are_its_compound_paths() {
        let mut s = session();
        let a = rect(&mut s, 0.0, 50.0);
        let b = rect(&mut s, 10.0, 20.0);
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        let r = s.execute("stitch.setType", &json!({"type": "tatami", "ids": [a]})).unwrap();
        assert_eq!(r["changed"], json!([c]));
        assert_eq!(props_of(&mut s, c)["kind"], json!("compound"));
    }
}
