use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, StitchType};
use vectorcraft_imageprep::{FUR_ANGLES, encode_png, synthetic_fur};

use crate::Session;

/// The synthetic fur picture written to a temp file of its own.
fn fur_file(name: &str, w: u32, h: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-photostitch-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fur.png");
    std::fs::write(&path, encode_png(&synthetic_fur(w, h, 2)).unwrap()).unwrap();
    path
}

fn run(s: &mut Session, id: &str, p: Value) -> Result<Value, String> {
    s.execute(id, &p).map_err(|e| e.to_string())
}

fn angle_error(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(180.0);
    d.min(180.0 - d)
}

#[test]
fn palette_reduces_the_photo_to_matched_threads() {
    let f = fur_file("palette", 240, 180);
    let mut s = Session::new();
    let r = run(&mut s, "photo.palette", json!({"file": f, "colors": 7})).unwrap();
    let colors = r["colors"].as_array().unwrap();
    assert!(colors.len() >= 2 && colors.len() <= 7, "{r}");
    let total: f64 = colors.iter().map(|c| c["coverage"].as_f64().unwrap()).sum();
    assert!((total - 1.0).abs() < 0.01, "{total}");
    // Most coverage first, each named by the starter chart.
    assert!(colors.windows(2).all(|w| w[0]["coverage"].as_f64() >= w[1]["coverage"].as_f64()));
    assert!(colors.iter().all(|c| c["code"].is_string() && c["thread"].as_str().unwrap().starts_with('#')));
    assert_eq!(r["chart"], "VectorCraft Starter");
    assert_eq!((r["width"].as_u64(), r["widthMm"].as_f64()), (Some(240), Some(100.0)));
}

#[test]
fn regions_are_editable_paths_sewn_as_tatami_at_their_direction() {
    let f = fur_file("regions", 240, 180);
    let mut s = Session::new();
    let r = run(&mut s, "photo.regions", json!({"file": f, "widthMm": 80})).unwrap();
    assert!(r["regions"].as_u64().unwrap() >= 2);
    let st = s.doc().unwrap();
    let group = st.doc.node(vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())).unwrap();
    assert_eq!(group.name.as_deref(), Some("Photo regions"));
    // The largest region of each half has the half's direction.
    let mut found = vec![];
    for g in group.children().unwrap() {
        for p in g.children().unwrap() {
            let NodeKind::Path { path, .. } = &p.kind else { panic!("not a path") };
            let props = p.stitch_props().unwrap();
            assert_eq!(props.kind, StitchType::Tatami);
            let b = path.bounds().unwrap();
            found.push((b.width() * b.height(), b.center().x, props.fill_angle.unwrap()));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let half = 80.0 * vectorcraft_stitch::PT_PER_MM / 2.0;
    let left = found.iter().find(|f| f.1 < half).unwrap();
    let right = found.iter().find(|f| f.1 > half).unwrap();
    assert!(angle_error(left.2, FUR_ANGLES.0) < 15.0, "{found:?}");
    assert!(angle_error(right.2, FUR_ANGLES.1) < 15.0, "{found:?}");
    // One undo step takes them away.
    run(&mut s, "edit.undo", json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())).is_none());
}

#[test]
fn the_direction_field_is_reported_and_drawn() {
    let f = fur_file("field", 240, 180);
    let out = f.with_file_name("field.png");
    let mut s = Session::new();
    let r = run(&mut s, "photo.directionField", json!({"file": f, "widthMm": 120, "cell": 4, "path": out})).unwrap();
    // 4 mm cells at 0.5 mm a pixel: 8 pixels.
    assert_eq!((r["cell"].as_f64(), r["cols"].as_u64(), r["rows"].as_u64()), (Some(8.0), Some(30), Some(23)));
    assert_eq!(r["angles"].as_array().unwrap().len(), 30 * 23);
    // Cell (5, 11): x 44, y 92 pixels, in the left half.
    let a = r["angles"][11 * 30 + 5].as_f64().unwrap();
    assert!(angle_error(a, FUR_ANGLES.0) < 10.0, "{a}");
    let png = std::fs::read(&out).unwrap();
    assert_eq!(&png[1..4], b"PNG");
    let inline = run(&mut s, "photo.directionField", json!({"file": f, "preview": true})).unwrap();
    assert!(inline["pngBase64"].is_string());
}

#[test]
fn to_stitches_opens_a_document_whose_export_is_the_plan() {
    let f = fur_file("tostitches", 200, 150);
    let mut s = Session::new();
    let r = run(&mut s, "photo.toStitches", json!({"file": f, "provider": "none", "widthMm": 60, "feather": 1.0})).unwrap();
    assert_eq!(r["fill"], "field");
    let stitches = r["stitches"].as_u64().unwrap();
    assert!(stitches > 1_000 && stitches < 100_000, "{r}");
    assert_eq!(s.doc().unwrap().doc.layers.len(), 2);
    // The export agrees with what the command reported.
    let info = run(&mut s, "stitch.info", json!({})).unwrap();
    assert_eq!(info["stitches"], r["stitches"]);
    // Tatami: editable regions, sewn by their stitch properties.
    let t = run(&mut s, "photo.toStitches", json!({"file": f, "provider": "nomodel", "widthMm": 60, "fill": "tatami"})).unwrap();
    assert_eq!((t["fill"].as_str(), t["provider"].as_str()), (Some("tatami"), Some("nomodel")));
    assert!(t["stitches"].as_u64().unwrap() > 1_000);
    assert_eq!(s.documents().len(), 2);
}

#[test]
fn to_stitches_refuses_what_it_cannot_do() {
    let f = fur_file("refuse", 120, 90);
    let mut s = Session::new();
    for (p, want) in [
        (json!({}), "required"),
        (json!({"file": f, "image": "AAAA"}), "not both"),
        (json!({"image": "%%%"}), "base64"),
        (json!({"file": f, "colors": 3}), "colors"),
        (json!({"file": f, "fill": "satin"}), "fill"),
        (json!({"file": f, "minLength": 5, "maxLength": 3}), "maxLength"),
        (json!({"file": f, "featherEdges": "middle"}), "featherEdges"),
        (json!({"file": f, "provider": "gemini"}), "provider"),
        (json!({"file": f, "provider": "none", "spacing": 0.2, "widthMm": 1000, "maxStitches": 1000}), "stitches"),
        (json!({"file": f, "widthMm": f64::MAX}), "widthMm"),
    ] {
        let e = run(&mut s, "photo.toStitches", p.clone()).unwrap_err();
        assert!(e.contains(want), "{p}: {e}");
    }
    assert!(s.documents().is_empty());
}

#[test]
fn the_synthetic_fur_is_written_on_demand() {
    let dir = std::env::temp_dir().join(format!("vc-photostitch-{}-synthetic", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("fur.png");
    let mut s = Session::new();
    let r = run(&mut s, "photo.syntheticFur", json!({"path": out, "width": 64, "height": 48})).unwrap();
    assert_eq!(r["angles"], json!([30.0, 120.0]));
    let img = vectorcraft_imageprep::Raster::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(img, synthetic_fur(64, 48, 1));
    assert!(run(&mut s, "photo.syntheticFur", json!({"width": 1e9})).is_err());
}
