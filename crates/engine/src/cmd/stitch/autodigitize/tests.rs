use serde_json::{Value, json};

use crate::Session;

/// Points per mm.
const MM: f64 = 72.0 / 25.4;

/// A document with filled shapes (no stroke) in `colour`: each `(x, y, w, h, ellipse)` in mm.
fn shapes(s: &mut Session, colour: &str, list: &[(f64, f64, f64, f64, bool)]) -> Vec<u64> {
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": colour})).unwrap();
    let mut ids = vec![];
    for &(x, y, w, h, ellipse) in list {
        s.execute("select.none", &json!({})).unwrap();
        let cmd = if ellipse { "shape.ellipse" } else { "shape.rectangle" };
        let id = s.execute(cmd, &json!({"x": x * MM, "y": y * MM, "width": w * MM, "height": h * MM})).unwrap()["id"].as_u64().unwrap();
        ids.push(id);
    }
    ids
}

fn kinds(r: &Value) -> Vec<String> {
    r["objects"].as_array().unwrap().iter().map(|o| o["kind"].as_str().unwrap().to_string()).collect()
}

#[test]
fn info_classes_shapes_by_width() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    let ids = shapes(&mut s, "#cc2222", &[(10.0, 10.0, 0.6, 30.0, false), (20.0, 10.0, 4.0, 30.0, false), (40.0, 10.0, 20.0, 20.0, true)]);
    let r = s.execute("stitch.autoDigitize.info", &json!({"ids": ids})).unwrap();
    assert_eq!(kinds(&r), ["run", "satin", "fill"], "{r}");
    // A wider satin limit makes the 20 mm disc... still a compact blob: a fill.
    let r = s.execute("stitch.autoDigitize.info", &json!({"ids": [ids[1]], "maxSatin": 3})).unwrap();
    assert_eq!(kinds(&r), ["fill"], "4 mm is wider than a 3 mm satin: {r}");
    for bad in [json!({"minColumn": 0.1}), json!({"maxSatin": 40}), json!({"minColumn": 2, "maxSatin": 2}), json!({"maxSatin": "wide"})] {
        let mut p = bad.clone();
        p["ids"] = json!(ids);
        assert!(s.execute("stitch.autoDigitize.info", &p).is_err(), "{p}");
    }
    assert!(s.execute("stitch.autoDigitize.info", &json!({"ids": [99999]})).is_err());
}

#[test]
fn auto_digitize_replaces_shapes_with_stitch_objects_in_one_undo_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    let ids = shapes(&mut s, "#2255cc", &[(10.0, 10.0, 4.0, 30.0, false), (30.0, 10.0, 20.0, 20.0, true)]);
    let before = s.execute("stitch.info", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let r = s.execute("stitch.autoDigitize", &json!({})).unwrap();
    assert_eq!((r["columns"].as_u64(), r["fills"].as_u64()), (Some(1), Some(1)), "{r}");
    let created: Vec<u64> = r["created"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(created.len(), 2);
    // The bar's group holds one satin column of two rails, in the source's colour.
    let props = s.execute("stitch.props", &json!({"ids": [created[0]]})).unwrap();
    assert_eq!(props["objects"][0]["kind"], json!("group"), "{props}");
    let info = s.execute("stitch.info", &json!({})).unwrap();
    assert!(info["stitches"].as_u64().unwrap() > 100, "{info}");
    assert_eq!(info["threads"][0]["rgb"], json!("#2255cc"));
    // Stitch objects are not digitized again.
    assert!(s.execute("stitch.autoDigitize", &json!({"ids": created})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("stitch.info", &json!({})).unwrap()["stitches"], before["stitches"]);
    // keepSource hides the source.
    let r = s.execute("stitch.autoDigitize", &json!({"ids": [ids[0]], "keepSource": true})).unwrap();
    assert_eq!(r["created"].as_array().unwrap().len(), 1);
}

#[test]
fn the_new_objects_take_the_sources_place_in_the_sewing_order() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    let ids = shapes(&mut s, "#000000", &[(10.0, 10.0, 4.0, 30.0, false), (30.0, 10.0, 4.0, 30.0, false)]);
    s.execute("stitch.sequence.set", &json!({"ids": [ids[1], ids[0]]})).unwrap();
    s.execute("stitch.autoDigitize", &json!({"ids": [ids[1]]})).unwrap();
    let l = s.execute("stitch.sequence.list", &json!({})).unwrap();
    let order: Vec<u64> = l["objects"].as_array().unwrap().iter().map(|i| i["id"].as_u64().unwrap()).collect();
    assert_eq!(order.len(), 2, "{l}");
    assert_eq!(order[1], ids[0], "the column is still sewn first: {l}");
}

#[test]
fn a_stroked_line_is_classed_by_its_weight() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    let mut kinds_of = vec![];
    for w in [1.0, 3.0, 12.0] {
        s.execute("select.none", &json!({})).unwrap();
        let id = s.execute("shape.line", &json!({"x1": 20, "y1": 20, "x2": 200, "y2": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("stroke.set", &json!({"weight": w * MM})).unwrap();
        let r = s.execute("stitch.autoDigitize.info", &json!({"ids": [id]})).unwrap();
        kinds_of.push(kinds(&r)[0].clone());
    }
    assert_eq!(kinds_of, ["run", "satin", "fill"]);
}

/// A PNG of a logo drawn in code: a red disc and a blue bar on white.
fn logo_png() -> Vec<u8> {
    vectorcraft_trace::Raster::from_fn(200, 120, |x, y| {
        let (fx, fy) = (f64::from(x), f64::from(y));
        if (fx - 50.0).hypot(fy - 60.0) < 35.0 {
            [200, 30, 30, 255]
        } else if (110.0..190.0).contains(&fx) && (50.0..62.0).contains(&fy) {
            [30, 60, 200, 255]
        } else {
            [255, 255, 255, 255]
        }
    })
    .encode_png()
}

#[test]
fn an_image_becomes_sorted_stitch_objects() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    let data = vectorcraft_format::base64_encode(&logo_png());
    let id = s.execute("file.place", &json!({"name": "logo.png", "dataBase64": data})).unwrap()["ids"][0].as_u64().unwrap();
    let r = s.execute("image.autoDigitize", &json!({"id": id, "colors": 3})).unwrap();
    assert_eq!(r["regions"], json!(2), "{r}");
    assert!(r["fills"].as_u64().unwrap() >= 1 && r["columns"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(r["colorChanges"], json!(1), "{r}");
    // The image is on a hidden template layer: not sewn.
    let info = s.execute("stitch.info", &json!({})).unwrap();
    assert!(info["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap_or("").contains("other kinds")), "{info}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.execute("image.autoDigitize", &json!({"id": id, "colors": 30})).is_err());
    assert!(s.execute("image.autoDigitize", &json!({"ids": []})).is_err());
}
