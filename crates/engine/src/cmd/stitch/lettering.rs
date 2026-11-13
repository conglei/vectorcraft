//! Embroidery lettering (E9): type is sewn as satin lettering by the export
//! ([`vectorcraft_stitch::lettering_of`]). `stitch.lettering.info` says what selected type sews
//! as; `text.toStitches` turns it into editable stitch objects (two-rail satin columns, runs and
//! fills), one undo step. The lettering parameters are set with `stitch.setParams`.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, StitchProps, StitchType};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_stitch::{
    FromDocOptions, MAX_STEPS, PT_PER_MM, THREAD_WIDTH_MM, inherited_props, letter_height, lettering_of, lettering_params, letters_of, mode_for,
    shape_letter, stats, text_mode,
};

use super::super::edit::selected_roots;
use super::super::fileio::stitchfile;
use super::super::typemenu::text_ids;
use super::super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_doc, targets};

const INFO: &str = "stitch.lettering.info";
const TO_STITCHES: &str = "text.toStitches";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stitch.lettering.info",
            "Lettering Info",
            [],
            None,
            "{ids?|id? (default: the selection; groups give the type inside them), options?: {…the stitch options of stitch.info: the document's defaults}} what each type object sews as (stitch type automatic, lettering or satin: satin lettering, running stitch under the run height; run: running stitch; tatami: filled letters; set the lettering parameters minColumn, runUnder, compensation, satinSpacing, runLength, underlay, letterTrims, letterJoin and bean with stitch.setParams) → {objects: [{id, heightMm (the tallest letter), mode: satin|run|fill|none, letters, columns (satin columns), runs (running stitch lines), fills (filled letters, and the wide parts of letters whose strokes are wider than satin: tatami under columns that reach 0.3 mm into it), trims, stitches (as stitch.info counts them for this object alone, lock stitches included)}]}; selecting no type is an error",
            has_doc,
            info
        ),
        cmd!(
            "text.toStitches",
            "Convert Type to Stitch Objects",
            ["Embroidery"],
            None,
            "{options?: {…stitch options, as stitch.lettering.info}} replace the selected type with the stitch objects its lettering is made of, a group per text object (stitch type lettering: sewn as one lettering object, its pieces joined by walks in lettering's order and trimmed per word, taking the type's place in the sewing order) holding a group per word, each holding a group per letter: a filled part's outline first (tatami), each satin column a compound path of two open rails (stitch type satin, its compensation, spacing and underlay set), each running stitch line an open path (run), in the letters' thread colour; a column whose rails are moved apart from the rest is reached by a walk inside the letter or a trim; one undo step → {ids: [group id…], columns, runs, fills}",
            has_type,
            to_stitches
        ),
    ]
}

/// Enabled with type selected.
fn has_type(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document")?;
    let roots: Vec<NodeId> = st.selection.objects.clone();
    if text_ids(&st.doc, &roots).is_empty() { Err("select type".into()) } else { Ok(()) }
}

/// The type objects `p` names (or the selection), with the type inside named groups.
fn text_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let ids = targets(s, p)?;
    for id in &ids {
        d.node(*id).ok_or_else(|| bad(cmd, format!("no object {}", id.0)))?;
    }
    let texts = text_ids(d, &ids);
    if texts.is_empty() {
        return Err(bad(cmd, "select type"));
    }
    Ok(texts)
}

/// The document's stitch options `options` in `p` (the export's when not given).
fn options(p: &Value, cmd: &str) -> Result<(Value, FromDocOptions)> {
    let options = match p.get("options") {
        None | Some(Value::Null) => json!({}),
        Some(o) if o.is_object() => o.clone(),
        Some(o) => return Err(bad(cmd, format!("options must be an object of stitch options, not {o}"))),
    };
    let o = stitchfile::from_doc_options(cmd, &options)?;
    Ok((options, o))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let (raw, o) = options(p, INFO)?;
    let ids = text_targets(s, p, INFO)?;
    let d: &Document = &s.doc()?.doc;
    let mut objects = vec![];
    for id in ids {
        let (Some(n), Some((props, _))) = (d.node(id), inherited_props(d, id)) else { continue };
        let lettered = lettering_of(n, &props, &o, MAX_STEPS).map_err(|e| bad(INFO, e))?;
        let mut v =
            json!({"id": id.0, "heightMm": 0.0, "mode": "none", "letters": 0, "columns": 0, "runs": 0, "fills": 0, "trims": 0, "stitches": 0});
        if let Some(l) = lettered {
            v["heightMm"] = json!((l.height_mm * 100.0).round() / 100.0);
            v["mode"] = json!(l.mode.id());
            v["letters"] = json!(l.letters.len());
            v["columns"] = json!(l.letters.iter().map(|x| x.columns).sum::<usize>());
            v["runs"] = json!(l.letters.iter().map(|x| x.run_lines).sum::<usize>());
            v["fills"] = json!(l.letters.iter().map(|x| x.fills).sum::<usize>());
            if l.letters.iter().any(|x| !x.runs.is_empty())
                && let Ok((plan, _)) = stitchfile::plan(INFO, d, &raw, "dst", Some(vec![id]))
            {
                let st = stats(&plan);
                v["trims"] = json!(st.trims);
                v["stitches"] = json!(st.stitches);
            }
        }
        objects.push(v);
    }
    Ok(json!({ "objects": objects }))
}

/// A line in mm as an open or closed subpath in document points.
pub(super) fn subpath(pts: &[(f64, f64)], closed: bool) -> SubPath {
    let pts: Vec<Point> = pts.iter().map(|q| Point::new(q.0 * PT_PER_MM, q.1 * PT_PER_MM)).collect();
    SubPath::polyline(&pts, closed)
}

/// A stitch object: a path, or a compound path of several subpaths, painted and with `props`.
pub(super) fn stitch_object(d: &mut Document, subpaths: Vec<SubPath>, look: Appearance, props: StitchProps) -> Node {
    let id = d.alloc_id();
    let mut n = if subpaths.len() == 1 {
        Node::path(id, PathData { subpaths }, look)
    } else {
        let children = subpaths.into_iter().map(|sp| Arc::new(Node::path(d.alloc_id(), PathData::single(sp), Appearance::default()))).collect();
        let mut c = Node::new(id, NodeKind::Compound { children, rule: vectorcraft_geom::FillRule::NonZero });
        c.appearance = look.into();
        c
    };
    n.set_stitch_props(props);
    n
}

fn to_stitches(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, o) = options(p, TO_STITCHES)?;
    let roots = selected_roots(s)?;
    let doc = &s.doc()?.doc;
    let texts = text_ids(doc, &roots);
    if texts.is_empty() {
        return Err(bad(TO_STITCHES, "select type"));
    }
    // What each text object becomes, worked out before the edit.
    let mut made = vec![];
    let (mut columns, mut runs, mut fills) = (0usize, 0usize, 0usize);
    for tid in &texts {
        let (Some(n), Some((props, _))) = (doc.node(*tid), inherited_props(doc, *tid)) else { continue };
        let NodeKind::Text(t) = &n.kind else { continue };
        let Some(forced) = text_mode(props.kind) else { continue };
        let lp = lettering_params(&props, &o);
        let letters = letters_of(t).map_err(|e| bad(TO_STITCHES, e))?;
        let mode = forced.unwrap_or_else(|| mode_for(letter_height(&letters), &lp));
        let mut shapes = vec![];
        for l in &letters {
            let shape = shape_letter(&l.rings, mode, &lp).map_err(|e| bad(TO_STITCHES, e))?;
            columns += shape.columns.len();
            runs += shape.runs.len();
            fills += usize::from(!shape.fill.is_empty());
            shapes.push((l.rgb, l.word, shape));
        }
        made.push((*tid, n.name.clone(), lp, n.stitch_props().copied().unwrap_or_default(), shapes));
    }
    if made.is_empty() {
        return Err(bad(TO_STITCHES, "the selected type is not sewn (its stitch type is none or manual)"));
    }
    let ids = s.edit("Convert Type to Stitch Objects", |d, sel| {
        let mut out = vec![];
        for (tid, name, lp, lettering_props, shapes) in made {
            let Some((parent, index, _)) = d.position(tid) else { continue };
            // A group per word, holding a group per letter.
            let mut words: Vec<(usize, Vec<Arc<Node>>)> = vec![];
            for ([r, g, b], word, shape) in shapes {
                let thread = Paint::solid(Color::rgb8(r, g, b));
                let stroke = Appearance::basic(Paint::None, thread.clone(), THREAD_WIDTH_MM * PT_PER_MM);
                let mut parts: Vec<Arc<Node>> = vec![];
                // The fill first, as lettering sews it.
                if !shape.fill.is_empty() {
                    let props = StitchProps { kind: StitchType::Tatami, ..Default::default() };
                    let rings = shape.fill.iter().map(|r| subpath(r, true)).collect();
                    parts.push(Arc::new(stitch_object(d, rings, Appearance::basic(thread.clone(), Paint::None, 1.0), props)));
                }
                for c in &shape.columns {
                    let props = StitchProps {
                        kind: StitchType::Satin,
                        satin_spacing: Some(lp.spacing),
                        compensation: Some(lp.compensation),
                        underlay: Some(lp.underlay),
                        ..Default::default()
                    };
                    parts.push(Arc::new(stitch_object(d, vec![subpath(&c.left, false), subpath(&c.right, false)], stroke.clone(), props)));
                }
                for line in &shape.runs {
                    let props = StitchProps { kind: StitchType::Run, run_length: Some(lp.run_length), ..Default::default() };
                    parts.push(Arc::new(stitch_object(d, vec![subpath(line, false)], stroke.clone(), props)));
                }
                if parts.is_empty() {
                    continue;
                }
                let letter = Arc::new(Node::group(d.alloc_id(), parts));
                match words.last_mut() {
                    Some((w, letters)) if *w == word => letters.push(letter),
                    _ => words.push((word, vec![letter])),
                }
            }
            d.remove(tid)?;
            if words.is_empty() {
                continue;
            }
            let words: Vec<Arc<Node>> = words.into_iter().map(|(_, letters)| Arc::new(Node::group(d.alloc_id(), letters))).collect();
            let gid = d.alloc_id();
            let mut g = Node::group(gid, words);
            g.name = Some(name.unwrap_or_else(|| "Lettering".into()));
            // Sewn as one lettering object, as the type was: its pieces joined by walks and
            // trimmed per word, with the type's lettering settings.
            g.set_stitch_props(StitchProps {
                kind: StitchType::Lettering,
                entry: None,
                exit: None,
                join_entry: None,
                join_exit: None,
                overlap: None,
                ..lettering_props
            });
            d.insert(parent, index, g)?;
            super::autodigitize::replace_in_order(d, tid, &[gid]);
            out.push(gid);
        }
        if out.is_empty() {
            return Err(EngineError::Other("Convert Type to Stitch Objects: the type has no letters to sew".into()));
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "columns": columns, "runs": runs, "fills": fills }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    /// A document with "Hello" at a font size of `mm` millimetres; its id.
    fn hello(s: &mut Session, text: &str, mm: f64) -> u64 {
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        let size = mm * 72.0 / 25.4;
        s.execute("text.create", &json!({"x": 40, "y": 120, "text": text, "size": size})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn info_says_what_type_sews_as() {
        let mut s = Session::new();
        let id = hello(&mut s, "Hello", 15.0);
        let r = s.execute("stitch.lettering.info", &json!({"ids": [id]})).unwrap();
        let o = &r["objects"][0];
        assert_eq!((o["mode"].as_str(), o["letters"].as_u64()), (Some("satin"), Some(5)), "{r}");
        assert!(o["columns"].as_u64().unwrap() >= 5);
        assert_eq!(o["trims"], json!(0), "one word: no trims between letters");
        assert!(o["stitches"].as_u64().unwrap() > 400);
        // The same counts as stitch.info for the document.
        let info = s.execute("stitch.info", &json!({})).unwrap();
        assert_eq!(info["stitches"], o["stitches"]);
        assert!(info["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap_or("").contains("not stitched")), "{info}");
        // Small type is running stitch; trims per letter through stitch.setParams.
        let small = hello(&mut s, "Test", 5.0);
        assert_eq!(s.execute("stitch.lettering.info", &json!({"ids": [small]})).unwrap()["objects"][0]["mode"], json!("run"));
        s.execute("stitch.setParams", &json!({"ids": [small], "params": {"letterTrims": "letter", "runUnder": 2}})).unwrap();
        let o = s.execute("stitch.lettering.info", &json!({"ids": [small]})).unwrap()["objects"][0].clone();
        assert_eq!((o["mode"].as_str(), o["trims"].as_u64()), (Some("satin"), Some(3)), "{o}");
        // Not type.
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        assert!(s.execute("stitch.lettering.info", &json!({})).is_err());
        assert!(s.execute("stitch.lettering.info", &json!({"ids": [99999]})).is_err());
    }

    #[test]
    fn type_becomes_stitch_objects_in_one_undo_step() {
        let mut s = Session::new();
        let id = hello(&mut s, "Hi", 15.0);
        let before = s.execute("stitch.info", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let r = s.execute("text.toStitches", &json!({})).unwrap();
        assert!(r["columns"].as_u64().unwrap() >= 3, "{r}");
        let g = r["ids"][0].as_u64().unwrap();
        let props = s.execute("stitch.props", &json!({"ids": [g]})).unwrap();
        assert_eq!(props["objects"][0]["kind"], json!("group"));
        let after = s.execute("stitch.info", &json!({})).unwrap();
        // About the same stitches: the columns sewn as two-rail satin.
        let (a, b) = (before["stitches"].as_f64().unwrap(), after["stitches"].as_f64().unwrap());
        assert!((b / a - 1.0).abs() <= 0.05, "{a} then {b}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.info", &json!({})).unwrap()["stitches"], before["stitches"]);
        // Nothing selected: refused.
        s.execute("select.none", &json!({})).unwrap();
        assert!(s.execute("text.toStitches", &json!({})).is_err());
    }

    /// STI-38: converted type still sews as one lettering object: its columns joined by walks in
    /// lettering's order and trimmed per word, so "Hello" sews with no trim, as the type does
    /// (it had 50 trims, each column sewn as an object of its own).
    #[test]
    fn converted_hello_sews_as_the_type_does() {
        let mut s = Session::new();
        let id = hello(&mut s, "Hello", 15.0);
        let live = s.execute("stitch.info", &json!({})).unwrap();
        assert_eq!(live["trims"], json!(0), "{live}");
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let r = s.execute("text.toStitches", &json!({})).unwrap();
        let g = r["ids"][0].as_u64().unwrap();
        assert_eq!(s.execute("stitch.props", &json!({"ids": [g]})).unwrap()["objects"][0]["own"]["type"], json!("lettering"));
        let made = s.execute("stitch.info", &json!({})).unwrap();
        assert_eq!(made["trims"], live["trims"], "{made}");
        let (a, b) = (live["stitches"].as_f64().unwrap(), made["stitches"].as_f64().unwrap());
        assert!((b / a - 1.0).abs() <= 0.05, "{a} live, {b} converted");
        // One object in the sewing order, in the type's place.
        let seq = s.execute("stitch.sequence.list", &json!({})).unwrap();
        assert_eq!(seq["objects"].as_array().map(Vec::len), Some(1), "{seq}");
        // Two words: one trim between them, as the type has.
        let two = hello(&mut s, "Hello you", 15.0);
        let live = s.execute("stitch.info", &json!({})).unwrap()["trims"].clone();
        s.execute("select.set", &json!({"ids": [two]})).unwrap();
        s.execute("text.toStitches", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.info", &json!({})).unwrap()["trims"], live);
    }
}
