//! Embroidery setup: the hoop and the fabric of a design together (`stitch.setup`), as New
//! Document (`file.new {hoop, fabric}`) and Document Setup (`document.setup {embroidery}`) set them.
//!
//! A hoop is given as a library id (`"rect-130x180"` or `{id}`), as a custom size
//! (`{widthMm, heightMm, shape?, name?}`) or as `null` for none. `stitch.hoop.get` replies with a
//! hoop in the object form, so a reply can be passed back. The fabric is `#rrggbb`, or `null` for
//! the default; the default given as a colour is kept as no colour, so a reply passed back changes
//! nothing.

use serde_json::{Map, Value, json};
use vectorcraft_doc::stitch_design::{DEFAULT_FABRIC, MAX_HOOP_NAME, parse_rgb_hex, rgb_hex};
use vectorcraft_doc::{Document, StitchDesign};
use vectorcraft_geom::Rect;
use vectorcraft_stitch::hoop as hoops;
use vectorcraft_stitch::{DEFAULT_MARGIN_MM, Hoop, HoopShape, PT_PER_MM, Placement};

use super::super::{CommandSpec, Result, Session, bad, cmd, has_doc, number_or, text_param};
use super::production::{hoop_center, hoop_json};

const SETUP: &str = "stitch.setup";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "stitch.setup",
        "Embroidery Setup",
        [],
        None,
        "{hoop?: a hoop id of stitch.hoops.list | {id} | {widthMm, heightMm (a custom hoop, mm, 10–2000), shape?: rectangle (default) | round, name?} | null (no hoop), fabric?: \"#rrggbb\" (the colour the stitches are previewed on: the canvas's stitch view, Stitch Preview, stitch.preview and the worksheet) | null (the default, #ece8de), resizeArtboard?: false (the first artboard takes the hoop's size, about its centre, so the hoop's centre stays where it is), center?: (ignored: the reply can be passed back)} the design's hoop and fabric in one undo step, none when nothing changes; {} → the current ones → {hoop: (as stitch.hoop.get)|null, fabric: \"#rrggbb\", center: {x, y} (the hoop's centre on the page, document units: the first artboard's centre)}",
        has_doc,
        setup
    )]
}

/// The embroidery settings a command was given: each `None` leaves that setting as it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EmbroiderySetup {
    /// The hoop (`Some(None)`: none).
    pub hoop: Option<Option<Hoop>>,
    /// The fabric (`Some(None)`: the default).
    pub fabric: Option<Option<[u8; 3]>>,
    /// The first artboard takes the hoop's size.
    pub resize_artboard: bool,
}

/// A custom hoop from `o`'s `widthMm`, `heightMm`, `shape` and `name`.
pub(crate) fn custom_hoop(o: &Value, cmd: &str) -> Result<Hoop> {
    let w = number_or(o, "widthMm", f64::NAN, cmd)?;
    let h = number_or(o, "heightMm", f64::NAN, cmd)?;
    let shape = match text_param(o, "shape", cmd)? {
        None => HoopShape::Rectangle,
        Some(v) => HoopShape::parse(v).ok_or_else(|| bad(cmd, format!("shape `{v}`: rectangle or round")))?,
    };
    let name = text_param(o, "name", cmd)?.unwrap_or("");
    if name.chars().count() > MAX_HOOP_NAME {
        return Err(bad(cmd, "the hoop's name is too long"));
    }
    hoops::custom(name, w, h, shape).map_err(|e| bad(cmd, e))
}

/// The library hoop `id` names.
fn library_hoop(id: &str, cmd: &str) -> Result<Hoop> {
    hoops::find(id).ok_or_else(|| bad(cmd, format!("no hoop `{id}`: stitch.hoops.list lists them")))
}

/// A `hoop` param: an id, `{id}` (other keys ignored, so a reply passes back), a custom size, or
/// null (none).
pub(crate) fn hoop_param(v: &Value, cmd: &str) -> Result<Option<Hoop>> {
    match v {
        Value::Null => Ok(None),
        Value::String(id) => library_hoop(id, cmd).map(Some),
        Value::Object(o) => match o.get("id") {
            Some(Value::String(id)) => library_hoop(id, cmd).map(Some),
            None | Some(Value::Null) => custom_hoop(v, cmd).map(Some),
            Some(other) => Err(bad(cmd, format!("hoop.id must be a hoop id, not {other}"))),
        },
        other => Err(bad(cmd, format!("hoop must be a hoop id, {{id}}, {{widthMm, heightMm, shape?, name?}} or null, not {other}"))),
    }
}

/// A `fabric` param: `#rrggbb`, or null for the default (the default as a colour is the default).
pub(crate) fn fabric_param(v: &Value, cmd: &str) -> Result<Option<[u8; 3]>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => {
            let c = parse_rgb_hex(s).filter(|_| s.starts_with('#')).ok_or_else(|| bad(cmd, format!("fabric `{s}`: a colour as #rrggbb")))?;
            Ok((c != DEFAULT_FABRIC).then_some(c))
        }
        other => Err(bad(cmd, format!("fabric must be a colour as #rrggbb or null, not {other}"))),
    }
}

impl EmbroiderySetup {
    /// The settings in `p`'s `hoop`, `fabric` and `resizeArtboard` (other keys are the caller's).
    pub(crate) fn parse(p: &Map<String, Value>, cmd: &str) -> Result<Self> {
        let resize_artboard = match p.get("resizeArtboard") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => return Err(bad(cmd, format!("resizeArtboard must be true or false, not {other}"))),
        };
        Ok(EmbroiderySetup {
            hoop: p.get("hoop").map(|v| hoop_param(v, cmd)).transpose()?,
            fabric: p.get("fabric").map(|v| fabric_param(v, cmd)).transpose()?,
            resize_artboard,
        })
    }

    /// What `doc` becomes: its sewing settings, and the first artboard's new rectangle when it is
    /// resized (and changes).
    pub(crate) fn result(&self, doc: &Document, cmd: &str) -> Result<(StitchDesign, Option<Rect>)> {
        let mut next = doc.stitch.clone();
        if let Some(h) = &self.hoop {
            next.hoop.clone_from(h);
        }
        if let Some(f) = self.fabric {
            next.fabric = f;
        }
        let mut board = None;
        if self.resize_artboard {
            let hoop = next.hoop().ok_or_else(|| bad(cmd, "resizeArtboard needs a hoop: give hoop or set one first"))?;
            let first = doc.artboards.first().ok_or_else(|| bad(cmd, "the document has no artboard to resize"))?;
            let (w, h) = (hoop.width_mm * PT_PER_MM, hoop.height_mm * PT_PER_MM);
            let c = first.rect.center();
            let r = Rect::new(c.x - w / 2.0, c.y - h / 2.0, c.x + w / 2.0, c.y + h / 2.0);
            if r != first.rect {
                board = Some(r);
            }
        }
        Ok((next, board))
    }
}

/// Put `stitch` and the first artboard's `board` into `d`.
pub(crate) fn apply(d: &mut Document, stitch: StitchDesign, board: Option<Rect>) {
    d.stitch = stitch;
    if let (Some(r), Some(first)) = (board, d.artboards.first_mut()) {
        first.rect = r;
    }
}

/// The design's hoop and fabric as `stitch.setup` reports them (and takes them back).
pub(crate) fn setup_json(doc: &Document) -> Value {
    let c = hoop_center(doc);
    json!({
        "hoop": doc.stitch.hoop().map_or(Value::Null, hoop_json),
        "fabric": rgb_hex(doc.stitch.fabric()),
        "center": {"x": c.x, "y": c.y},
    })
}

fn setup(s: &mut Session, p: &Value) -> Result<Value> {
    let empty = Map::new();
    let o = match p {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(bad(SETUP, "params must be an object")),
    };
    let doc = &s.doc()?.doc;
    let (next, board) = EmbroiderySetup::parse(o, SETUP)?.result(doc, SETUP)?;
    if next != doc.stitch || board.is_some() {
        s.edit("Embroidery Setup", |d, _| {
            apply(d, next, board);
            Ok(())
        })?;
    }
    Ok(setup_json(&s.doc()?.doc))
}

/// Where stitches spanning `bounds_mm` (left, top, right, bottom in document mm) lie in `doc`'s
/// hoop, centred on the first artboard, with the default margin: `None` without a hoop.
pub fn hoop_placement(doc: &Document, bounds_mm: (f64, f64, f64, f64)) -> Option<(Hoop, Placement)> {
    let hoop = doc.stitch.hoop()?.clone();
    let c = hoop_center(doc);
    let (cx, cy) = (c.x / PT_PER_MM, c.y / PT_PER_MM);
    let (x0, y0, x1, y1) = bounds_mm;
    let at = hoops::placement(&hoop, (x0 - cx, y0 - cy, x1 - cx, y1 - cy), DEFAULT_MARGIN_MM);
    Some((hoop, at))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        s
    }

    #[test]
    fn setup_sets_the_hoop_and_fabric_in_one_step_and_reports_them() {
        let mut s = session();
        let r = s.execute("stitch.setup", &json!({})).unwrap();
        assert_eq!(r, json!({"hoop": null, "fabric": "#ece8de", "center": {"x": 300.0, "y": 200.0}}));
        let undo = s.active().unwrap().history.undo.len();
        let r = s.execute("stitch.setup", &json!({"hoop": "round-150", "fabric": "#203040", "resizeArtboard": true})).unwrap();
        assert_eq!((r["hoop"]["id"].as_str(), r["fabric"].as_str()), (Some("round-150"), Some("#203040")));
        assert_eq!(s.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        // The artboard is the hoop's size about the same centre.
        let ab = s.active().unwrap().doc.artboards[0].rect;
        assert!((ab.width() - 150.0 * 72.0 / 25.4).abs() < 1e-6 && (ab.center().x - 300.0).abs() < 1e-9, "{ab:?}");
        assert_eq!(r["center"], json!({"x": 300.0, "y": 200.0}));
        // The reply passed back changes nothing.
        let again = s.execute("stitch.setup", &r).unwrap();
        assert_eq!(again, r);
        assert_eq!(s.active().unwrap().history.undo.len(), undo + 1);
        // A custom hoop, the default fabric as a colour and none.
        let r = s.execute("stitch.setup", &json!({"hoop": {"widthMm": 120, "heightMm": 90, "shape": "round"}, "fabric": "#ece8de"})).unwrap();
        assert_eq!((r["hoop"]["id"].clone(), r["hoop"]["shape"].as_str()), (Value::Null, Some("round")));
        assert_eq!(s.active().unwrap().doc.stitch.fabric, None);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.setup", &json!({})).unwrap()["fabric"], "#203040");
        let r = s.execute("stitch.setup", &json!({"hoop": null, "fabric": null})).unwrap();
        assert_eq!((r["hoop"].clone(), r["fabric"].as_str()), (Value::Null, Some("#ece8de")));
        for bad in [
            json!({"hoop": "nope"}),
            json!({"hoop": 3}),
            json!({"hoop": {"id": 4}}),
            json!({"hoop": {"widthMm": 5, "heightMm": 100}}),
            json!({"fabric": "red"}),
            json!({"fabric": "ece8de"}),
            json!({"fabric": [1, 2, 3]}),
            json!({"resizeArtboard": true}),
            json!({"resizeArtboard": "yes", "hoop": "square-100"}),
            json!([]),
        ] {
            assert!(s.execute("stitch.setup", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_new_design_takes_its_hoops_size_and_fabric_without_an_undo_step() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"hoop": "rect-130x180", "fabric": "#334455", "units": "Millimeters"})).unwrap();
        let st = s.active().unwrap();
        assert!(st.history.undo.is_empty(), "born with them");
        let ab = st.doc.artboards[0].rect;
        assert!((ab.width() - 130.0 * 72.0 / 25.4).abs() < 1e-6 && (ab.height() - 180.0 * 72.0 / 25.4).abs() < 1e-6, "{ab:?}");
        let r = s.execute("stitch.hoop.get", &json!({})).unwrap();
        assert_eq!((r["hoop"]["id"].as_str(), r["fabric"].as_str()), (Some("rect-130x180"), Some("#334455")));
        // A custom round hoop; a size given wins over the hoop's.
        s.execute("file.new", &json!({"hoop": {"widthMm": 150, "heightMm": 150, "shape": "round"}, "width": 600, "height": 600})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.artboards[0].rect.width(), 600.0);
        assert_eq!(st.doc.stitch.hoop().map(|h| h.shape), Some(vectorcraft_stitch::HoopShape::Round));
        assert_eq!(st.doc.stitch.fabric, None);
        for bad in [
            json!({"hoop": "nope"}),
            json!({"hoop": {"widthMm": 1}}),
            json!({"fabric": "blue"}),
            json!({"hoop": "square-100", "resizeArtboard": true}),
        ] {
            assert!(s.execute("file.new", &bad).is_err(), "{bad}");
        }
        // The hoop list needs no document.
        let mut none = Session::new();
        let r = none.execute("stitch.hoops.list", &json!({})).unwrap();
        assert_eq!((r["hoops"].as_array().unwrap().len(), r["chosen"].clone()), (10, Value::Null));
    }

    /// A design in a 100 mm square hoop: a stroked 30 mm square centred, then moved over the edge,
    /// then into the margin. Art that sews nothing outside the hoop does not count.
    #[test]
    fn the_checker_reports_stitches_outside_the_hoop_where_they_are() {
        let mm = 72.0 / 25.4;
        let mut s = Session::new();
        s.execute("file.new", &json!({"hoop": "square-100"})).unwrap();
        let outside = |s: &mut Session| {
            let r = s.execute("stitch.check", &json!({})).unwrap();
            r["findings"].as_array().unwrap().iter().find(|f| f["code"] == "outsideHoop").cloned()
        };
        // Unstroked art far outside the hoop: nothing to sew there.
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        s.execute("paint.setFill", &json!({"none": true})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 2000, "y": 2000, "width": 50, "height": 50})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#aa0000"})).unwrap();
        let id =
            s.execute("shape.rectangle", &json!({"x": 35.0 * mm, "y": 35.0 * mm, "width": 30.0 * mm, "height": 30.0 * mm})).unwrap()["id"].clone();
        assert_eq!(outside(&mut s), None, "inside the field");
        // 40 mm to the right: from 75 to 105 mm, over the edge at 100 mm.
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        s.execute("object.move", &json!({"dx": 40.0 * mm, "dy": 0})).unwrap();
        let f = outside(&mut s).expect("over the edge");
        assert_eq!(f["level"], "error", "{f}");
        assert!(f["message"].as_str().unwrap().contains("Center Design in Hoop"), "{f}");
        assert!(f["where"]["x"].as_f64().unwrap() > 100.0 * mm, "{f}");
        // Back 12 mm: 63 to 93 mm, inside the hoop but within its 10 mm margin.
        s.execute("object.move", &json!({"dx": -12.0 * mm, "dy": 0})).unwrap();
        assert_eq!(outside(&mut s).expect("in the margin")["level"], "warning");
        // A smaller margin clears it; centring clears it too.
        let r = s.execute("stitch.check", &json!({"marginMm": 5})).unwrap();
        assert!(!r["findings"].as_array().unwrap().iter().any(|f| f["code"] == "outsideHoop"), "{r}");
        s.execute("stitch.hoop.center", &json!({})).unwrap();
        assert_eq!(outside(&mut s), None);
    }

    #[test]
    fn document_setup_sets_the_embroidery_in_its_one_step() {
        let mut s = session();
        let undo = s.active().unwrap().history.undo.len();
        let r = s
            .execute(
                "document.setup",
                &json!({"units": "Millimeters", "embroidery": {"hoop": "square-200", "fabric": "#102030", "resizeArtboard": true}}),
            )
            .unwrap();
        assert_eq!(s.active().unwrap().history.undo.len(), undo + 1);
        assert_eq!((r["embroidery"]["hoop"]["id"].as_str(), r["embroidery"]["fabric"].as_str()), (Some("square-200"), Some("#102030")));
        assert!((s.active().unwrap().doc.artboards[0].rect.width() - 200.0 * 72.0 / 25.4).abs() < 1e-6);
        // Its reply passed back changes nothing.
        s.execute("document.setup", &r).unwrap();
        assert_eq!(s.active().unwrap().history.undo.len(), undo + 1);
        assert!(s.execute("document.setup", &json!({"embroidery": 5})).is_err());
        assert!(s.execute("document.setup", &json!({"embroidery": {"hoop": "x"}})).is_err());
    }

    #[test]
    fn the_fabric_survives_save_and_open() {
        let mut s = session();
        s.execute("stitch.setup", &json!({"hoop": "rect-130x180", "fabric": "#1a2b3c"})).unwrap();
        let dir = std::env::temp_dir().join(format!("vc-fabric-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fabric.vectorcraft");
        s.execute("file.saveAs", &json!({"path": path.to_str().unwrap()})).unwrap();
        let mut t = Session::new();
        t.execute("document.open", &json!({"path": path.to_str().unwrap()})).unwrap();
        let r = t.execute("stitch.setup", &json!({})).unwrap();
        assert_eq!((r["fabric"].as_str(), r["hoop"]["id"].as_str()), (Some("#1a2b3c"), Some("rect-130x180")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
