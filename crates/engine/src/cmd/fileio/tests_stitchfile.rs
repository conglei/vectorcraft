//! DST and EXP stitch files through the commands: export with the stitch options, open by content
//! or extension, `stitch.info`, refused options and the format table.

use serde_json::{Value, json};

use super::*;

/// Document points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A 100 × 50 pt rectangle stroked red, without a fill.
fn red_rectangle() -> Session {
    let mut s = session();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    s
}

/// `params` with `extra`'s keys added.
fn with(params: Value, extra: &Value) -> Value {
    let mut p = params;
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    p
}

/// Exports the rectangle as `format`, opens the file again as `name` and compares the stitch
/// counts `stitch.info` gives for both documents.
fn round_trip(format: &str, name: &str, magic: Option<&[u8]>) {
    let mut s = red_rectangle();
    let options = json!({"format": format, "locks": false});
    let r = s.execute("document.export", &with(json!({"format": format}), &options)).unwrap();
    assert_eq!(r["format"], format);
    let bytes = b64(&r);
    if let Some(magic) = magic {
        assert!(bytes.starts_with(magic), "{format}: {:?}", &bytes[..bytes.len().min(8)]);
    }
    let before = s.execute("stitch.info", &options).unwrap();
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    let after = s.execute("stitch.info", &options).unwrap();
    assert!(before["stitches"].as_u64().unwrap() > 10, "{before}");
    assert_eq!(after["stitches"], before["stitches"], "before {before}\nafter {after}");
}

#[test]
fn a_stroked_rectangle_exports_and_reopens() {
    round_trip("dst", "a.dst", Some(b"LA:"));
    // A DST file opens by its content, whatever its name.
    let mut s = red_rectangle();
    let bytes = b64(&s.execute("document.export", &json!({"format": "dst"})).unwrap());
    let r = s.execute("document.open", &json!({"name": "a.bin", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert_eq!(r["format"], "dst");
}

#[test]
fn exp_exports_by_format_and_opens_by_extension() {
    round_trip("exp", "a.exp", None);
    let mut s = red_rectangle();
    let r = s.execute("document.export", &json!({"format": "exp"})).unwrap();
    assert!(!b64(&r).starts_with(b"LA:"), "not a DST file");
}

#[test]
fn stitch_info_reports_counts_and_size() {
    let mut s = session();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("shape.line", &json!({"x1": 0, "y1": 100, "x2": 100.0 * PT_PER_MM, "y2": 100})).unwrap();
    let r = s.execute("stitch.info", &json!({"locks": false, "stitchLength": 2.5})).unwrap();
    assert_eq!(r["stitches"], 41, "{r}");
    assert_eq!((r["jumps"].as_u64(), r["trims"].as_u64(), r["colorChanges"].as_u64()), (Some(0), Some(0), Some(0)), "{r}");
    assert_eq!((r["widthMm"].as_f64(), r["heightMm"].as_f64()), (Some(100.0), Some(0.0)), "{r}");
    let threads = r["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "{r}");
    assert_eq!(threads[0]["rgb"], "#000000");
    assert_eq!(threads[0]["stitches"], 41);
    assert!((threads[0]["lengthMm"].as_f64().unwrap() - 100.0).abs() < 1e-9, "{r}");
    assert!(r["warnings"].is_array(), "{r}");
}

#[test]
fn stitch_info_takes_the_selection() {
    let mut s = session();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("shape.line", &json!({"x1": 0, "y1": 100, "x2": 100.0 * PT_PER_MM, "y2": 100})).unwrap();
    let short = s.execute("shape.line", &json!({"x1": 0, "y1": 200, "x2": 10.0 * PT_PER_MM, "y2": 200})).unwrap()["id"].clone();
    let all = s.execute("stitch.info", &json!({"locks": false})).unwrap();
    assert_eq!(all["stitches"], 41 + 5, "{all}");
    s.execute("select.set", &json!({"ids": [short]})).unwrap();
    let selected = s.execute("stitch.info", &json!({"locks": false, "selectedOnly": true})).unwrap();
    assert_eq!(selected["stitches"], 5, "{selected}");
    // The export writes the same selection.
    s.execute("document.export", &json!({"format": "dst", "locks": false, "selectedOnly": true})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let e = s.execute("stitch.info", &json!({"selectedOnly": true})).unwrap_err().to_string();
    assert!(e.contains("select"), "{e}");
}

#[test]
fn bad_stitch_options_are_refused() {
    let mut s = red_rectangle();
    for (p, key) in [
        (json!({"stitchLength": 0}), "stitchLength"),
        (json!({"stitchLength": -1}), "stitchLength"),
        (json!({"stitchLength": "2"}), "stitchLength"),
        // JSON has no infinite numbers: the largest ones stand in for them.
        (json!({"stitchLength": 1e300}), "stitchLength"),
        (json!({"minStitch": 3}), "minStitch"),
        (json!({"maxStitch": 12.5}), "maxStitch"),
        (json!({"maxStitch": 0.5}), "maxStitch"),
        (json!({"jumpsPerTrim": 1}), "jumpsPerTrim"),
        (json!({"jumpsPerTrim": 9}), "jumpsPerTrim"),
        (json!({"jumpsPerTrim": 2.5}), "jumpsPerTrim"),
        (json!({"machine": "x"}), "machine"),
        (json!({"mode": "x"}), "mode"),
        (json!({"start": "x"}), "start"),
        (json!({"locks": "yes"}), "locks"),
        (json!({"name": 3}), "name"),
        (json!({"nope": 1}), "nope"),
    ] {
        for id in ["document.export", "stitch.info"] {
            let e = s.execute(id, &with(json!({"format": "dst"}), &p)).unwrap_err().to_string();
            assert!(e.contains(key), "{id} {p}: {e}");
        }
    }
    // EXP takes stitches up to 12.7 mm.
    s.execute("document.export", &json!({"format": "exp", "maxStitch": 12.5})).unwrap();
    s.execute("stitch.info", &json!({"format": "exp", "maxStitch": 12.7})).unwrap();
    let e = s.execute("stitch.info", &json!({"format": "pdf"})).unwrap_err().to_string();
    assert!(e.contains("dst") && e.contains("exp"), "{e}");
    // The options a file opens with.
    let bytes = b64(&s.execute("document.export", &json!({"format": "dst"})).unwrap());
    let data = vectorcraft_format::base64_encode(&bytes);
    s.execute("document.open", &json!({"name": "a.dst", "dataBase64": data, "stitch": {"jumpsPerTrim": 4}})).unwrap();
    for bad in [json!({"jumpsPerTrim": 1}), json!({"jumpsPerTrim": 9}), json!({"jumpsPerTrim": "3"}), json!({"nope": 1}), json!(3)] {
        let e = s.execute("document.open", &json!({"name": "a.dst", "dataBase64": data, "stitch": bad})).unwrap_err().to_string();
        assert!(e.contains("stitch") || e.contains("jumpsPerTrim"), "{bad}: {e}");
    }
}

#[test]
fn an_empty_document_is_an_error_not_an_empty_file() {
    let mut s = session();
    for id in ["document.export", "stitch.info"] {
        for format in ["dst", "exp"] {
            let e = s.execute(id, &json!({"format": format})).unwrap_err().to_string();
            assert!(e.contains("nothing to stitch"), "{id} {format}: {e}");
        }
    }
}

#[test]
fn a_file_of_too_many_colour_changes_is_refused() {
    // EXP colour change records, then one stitch.
    let mut bytes: Vec<u8> = [0x80, 0x01, 0x00, 0x00].repeat(10_001);
    bytes.extend([0x0A, 0x00]);
    let mut s = Session::new();
    let e = s.execute("document.open", &json!({"name": "a.exp", "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap_err().to_string();
    assert!(e.contains("too many colour changes"), "{e}");
}

#[test]
fn the_formats_are_listed() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    for (id, label, max) in [("dst", "DST stitch file", 12.1), ("exp", "EXP stitch file", 12.7)] {
        let f = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap_or_else(|| panic!("{id} not listed"));
        assert_eq!((f["read"].as_bool(), f["write"].as_bool(), f["raster"].as_bool()), (Some(true), Some(true), Some(false)), "{f}");
        assert_eq!(f["label"], label);
        assert_eq!(f["extensions"], json!([id]));
        assert_eq!(f["options"]["machine"]["default"], "commercial15");
        assert_eq!(f["options"]["jumpsPerTrim"]["type"], "integer");
        assert_eq!(f["options"]["name"]["default"], Value::Null);
        assert!(f["options"]["maxStitch"]["description"].as_str().unwrap().contains(&max.to_string()), "{f}");
        assert!(r["readable"].as_array().unwrap().contains(&json!(id)) && r["writable"].as_array().unwrap().contains(&json!(id)));
        assert!(OPEN_EXTS.contains(&id) && PLACE_EXTS.contains(&id) && UNASSOCIATED_EXTS.contains(&id), "{id}");
        assert_eq!(format_for_name(&format!("a.{id}")).map(|f| f.id), Some(id));
    }
    assert_eq!(detect("a.bin", b"LA:design").map(|f| f.id), Some("dst"), "DST by its content");
    assert_eq!(detect("a.exp", &[0x0A, 0x00]).map(|f| f.id), Some("exp"), "EXP by its extension");
}

#[test]
fn the_calibration_design_stitches_as_documented() {
    let mut s = session();
    let before = s.execute("stitch.info", &json!({})).unwrap_err().to_string();
    assert!(!before.is_empty());
    let r = s.execute("stitch.calibration", &json!({})).unwrap();
    assert!(r["id"].as_u64().is_some(), "{r}");
    let info = s.execute("stitch.info", &json!({"locks": false})).unwrap();
    assert_eq!(info["colorChanges"], 2, "{info}");
    assert_eq!(info["widthMm"].as_f64(), Some(100.0), "{info}");
    assert_eq!(info["heightMm"].as_f64(), Some(100.0), "{info}");
    assert!(info["trims"].as_u64().unwrap() >= 12, "{info}");
    assert_eq!(info["warnings"].as_array().map(Vec::len), Some(0), "{info}");
    // One undo step removes the whole group.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.execute("stitch.info", &json!({})).is_err());
}
