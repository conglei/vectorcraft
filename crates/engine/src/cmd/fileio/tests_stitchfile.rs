//! DST and EXP stitch files through the commands: export with the stitch options, open by content
//! or extension, `stitch.info`, refused options and the format table.

use serde_json::{Value, json};

use vectorcraft_doc::NodeId;
use vectorcraft_stitch::PT_PER_MM;

use super::*;

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

/// The `ST` value of a DST header.
fn header_st(bytes: &[u8]) -> u64 {
    std::str::from_utf8(&bytes[23..30]).unwrap().trim().parse().unwrap()
}

/// The records of an EXP file: four bytes for a control record, two for a stitch.
fn exp_records(bytes: &[u8]) -> u64 {
    let (mut n, mut i) = (0, 0);
    while i < bytes.len() {
        i += if bytes[i] == 0x80 { 4 } else { 2 };
        n += 1;
    }
    n
}

/// The needle points of each block of `plan`, in order.
fn needle_points(plan: &vectorcraft_stitch::StitchPlan) -> Vec<Vec<vectorcraft_stitch::Pt>> {
    use vectorcraft_stitch::Step;
    plan.blocks.iter().map(|b| b.steps.iter().filter_map(|s| if let Step::Stitch(p) = s { Some(*p) } else { None }).collect()).collect()
}

/// The stitch plan in a DST or EXP file.
fn read_plan(format: &str, bytes: &[u8]) -> vectorcraft_stitch::StitchPlan {
    let o = Default::default();
    match format {
        "dst" => vectorcraft_embroidery::dst::read(bytes, &o),
        _ => vectorcraft_embroidery::exp::read(bytes, &o),
    }
    .unwrap()
    .plan
}

#[test]
fn an_opened_stitch_file_re_exports_its_needle_points_unchanged() {
    for format in ["dst", "exp"] {
        let mut s = session();
        s.execute("stitch.calibration", &json!({})).unwrap();
        let first = b64(&s.execute("document.export", &json!({"format": format})).unwrap());
        let data = vectorcraft_format::base64_encode(&first);
        s.execute("document.open", &json!({"name": format!("cal.{format}"), "dataBase64": data})).unwrap();
        // Default options: the file's own needle points, no second set of lock stitches.
        let info = s.execute("stitch.info", &json!({"format": format})).unwrap();
        let again = b64(&s.execute("document.export", &json!({"format": format})).unwrap());
        let (a, b) = (needle_points(&read_plan(format, &first)), needle_points(&read_plan(format, &again)));
        assert_eq!(a.len(), 3, "{format}");
        assert_eq!(a, b, "{format}: the needle points changed");
        assert_eq!(info["stitches"].as_u64(), Some(a.iter().map(Vec::len).sum::<usize>() as u64), "{format}: {info}");
        // Options given explicitly still apply.
        let changed = b64(&s.execute("document.export", &json!({"format": format, "mode": "run", "locks": true})).unwrap());
        assert_ne!(needle_points(&read_plan(format, &changed)), a, "{format}");
    }
}

const MIXED_LOCKS: &str =
    "lock stitches were added to every run, including those of the opened stitch file: pass locks: false to keep that file's runs unchanged";

/// Draws a 70 mm circle stroked green, without a fill, and returns its id.
fn green_circle(s: &mut Session) -> Value {
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#00a000"})).unwrap();
    let d = 70.0 * PT_PER_MM;
    s.execute("shape.ellipse", &json!({"x": 400, "y": 400, "width": d, "height": d})).unwrap()["id"].clone()
}

/// The stitch count `stitch.info` reply `r` gives the thread `rgb`.
fn thread_stitches(r: &Value, rgb: &str) -> u64 {
    r["threads"].as_array().unwrap().iter().find(|t| t["rgb"] == rgb).unwrap_or_else(|| panic!("no {rgb} in {r}"))["stitches"].as_u64().unwrap()
}

/// A session holding the calibration design opened from its DST file, and that file.
fn opened_calibration() -> (Session, Vec<u8>) {
    let mut s = session();
    s.execute("stitch.calibration", &json!({})).unwrap();
    let file = b64(&s.execute("document.export", &json!({"format": "dst"})).unwrap());
    s.execute("document.open", &json!({"name": "cal.dst", "dataBase64": vectorcraft_format::base64_encode(&file)})).unwrap();
    (s, file)
}

#[test]
fn art_drawn_beside_an_opened_stitch_file_is_stitched_as_in_a_fresh_document() {
    let mut fresh = session();
    green_circle(&mut fresh);
    let want = thread_stitches(&fresh.execute("stitch.info", &json!({})).unwrap(), "#00a000");
    assert!(want > 50, "{want}");
    let (mut s, file) = opened_calibration();
    s.execute("layer.new", &json!({"name": "Drawn"})).unwrap();
    green_circle(&mut s);
    let info = s.execute("stitch.info", &json!({})).unwrap();
    assert_eq!(thread_stitches(&info, "#00a000"), want, "{info}");
    assert!(info["warnings"].as_array().unwrap().contains(&json!(MIXED_LOCKS)), "{info}");
    // The file's own needle points stay as they were when locks are turned off, with no warning.
    let info = s.execute("stitch.info", &json!({"locks": false})).unwrap();
    assert!(!info["warnings"].as_array().unwrap().contains(&json!(MIXED_LOCKS)), "{info}");
    let again = b64(&s.execute("document.export", &json!({"format": "dst", "locks": false})).unwrap());
    // The circle moves the file's centre, so compare from the first needle point.
    let from_first = |blocks: Vec<Vec<vectorcraft_stitch::Pt>>| {
        let f = blocks[0][0];
        blocks.into_iter().map(|b| b.into_iter().map(|p| (p.x - f.x, p.y - f.y)).collect::<Vec<_>>()).collect::<Vec<_>>()
    };
    let (a, b) = (from_first(needle_points(&read_plan("dst", &file))), from_first(needle_points(&read_plan("dst", &again))));
    assert_eq!(b.len(), 4);
    assert_eq!(a[..], b[..3], "the file's own needle points changed");
}

#[test]
fn a_selection_beside_an_opened_stitch_file_reports_what_its_export_writes() {
    let (mut s, _) = opened_calibration();
    s.execute("layer.new", &json!({"name": "Drawn"})).unwrap();
    let id = green_circle(&mut s);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let info = s.execute("stitch.info", &json!({"selectedOnly": true})).unwrap();
    let bytes = b64(&s.execute("document.export", &json!({"format": "dst", "selectedOnly": true})).unwrap());
    assert_eq!(info["records"].as_u64(), Some(header_st(&bytes)), "{info}");
    // Art selected on the opened file's own layer keeps its needle points in both.
    let first = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id;
    s.execute("select.set", &json!({"ids": [first.0]})).unwrap();
    let info = s.execute("stitch.info", &json!({"selectedOnly": true})).unwrap();
    let bytes = b64(&s.execute("document.export", &json!({"format": "dst", "selectedOnly": true})).unwrap());
    assert_eq!(info["records"].as_u64(), Some(header_st(&bytes)), "{info}");
    assert_eq!(info["stitches"].as_u64(), Some(needle_points(&read_plan("dst", &bytes)).iter().map(Vec::len).sum::<usize>() as u64));
}

#[test]
fn stitch_info_counts_the_records_the_file_holds() {
    let mut s = session();
    s.execute("stitch.calibration", &json!({})).unwrap();
    let info = s.execute("stitch.info", &json!({})).unwrap();
    let bytes = b64(&s.execute("document.export", &json!({"format": "dst"})).unwrap());
    assert_eq!(info["records"].as_u64(), Some(header_st(&bytes)), "{info}");
    assert_eq!(info["records"].as_u64(), Some(((bytes.len() - 512) / 3) as u64), "{info}");
    let info = s.execute("stitch.info", &json!({"format": "exp"})).unwrap();
    let bytes = b64(&s.execute("document.export", &json!({"format": "exp"})).unwrap());
    assert_eq!(info["records"].as_u64(), Some(exp_records(&bytes)), "{info}");
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
    // The export writes the same selection: its file holds those stitches and records.
    let bytes = b64(&s.execute("document.export", &json!({"format": "dst", "locks": false, "selectedOnly": true})).unwrap());
    assert_eq!(header_st(&bytes), selected["records"].as_u64().unwrap(), "{selected}");
    let written = vectorcraft_embroidery::dst::read(&bytes, &Default::default()).unwrap().plan;
    assert_eq!(needle_points(&written).iter().map(Vec::len).sum::<usize>(), 5);
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
    let r = s.execute("stitch.calibration", &json!({"x": 10, "y": 20})).unwrap();
    let id = r["id"].as_u64().unwrap();
    let group = s.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
    assert_eq!(group.name.as_deref(), Some("Stitch Calibration"));
    assert_eq!(
        group.children().unwrap().iter().map(|c| c.name.clone().unwrap()).collect::<Vec<_>>(),
        ["square", "ticks", "F", "near pair", "far pair"]
    );
    // The design's top-left corner is at (x, y), and it is 100 mm square.
    let b = group.geometric_bounds().unwrap();
    for (got, want) in [(b.x0, 10.0), (b.y0, 20.0), (b.x1, 10.0 + 100.0 * PT_PER_MM), (b.y1, 20.0 + 100.0 * PT_PER_MM)] {
        assert!((got - want).abs() < 1e-9, "{b:?}");
    }
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

/// How many trims `plan` holds.
fn trim_count(plan: &vectorcraft_stitch::StitchPlan) -> usize {
    use vectorcraft_stitch::Step;
    plan.blocks.iter().flat_map(|b| b.steps.iter()).filter(|s| **s == Step::Trim).count()
}

/// The stitch plan in a DST or EXP file, read with `jumps` jump records in a row as a trim.
fn read_plan_with(format: &str, bytes: &[u8], jumps: u8) -> vectorcraft_stitch::StitchPlan {
    let o = vectorcraft_embroidery::ReadOptions { jumps_per_trim: jumps };
    match format {
        "dst" => vectorcraft_embroidery::dst::read(bytes, &o),
        _ => vectorcraft_embroidery::exp::read(bytes, &o),
    }
    .unwrap()
    .plan
}

/// A plan of stitches 3 mm apart with one explicit trim, two 30 mm jumps (no trim) and a colour
/// change, written as `format` with `jumps` jump records in a row for a trim.
fn travelling_file(format: &str, jumps: u8) -> Vec<u8> {
    use vectorcraft_embroidery::{WriteOptions, dst, exp};
    use vectorcraft_stitch::{Block, Pt, Step, StitchPlan, Thread};
    let line = |x0: i32, n: i32| (0..n).map(|i| Step::Stitch(Pt { x: x0 + 30 * i, y: 0 })).collect::<Vec<_>>();
    let mut a = line(0, 4);
    a.extend([Step::Trim, Step::Jump(Pt { x: 390, y: 0 })]);
    a.extend(line(390, 3));
    a.push(Step::Jump(Pt { x: 750, y: 0 }));
    a.extend(line(750, 3));
    let mut b = vec![Step::Jump(Pt { x: 0, y: 100 })];
    b.extend((0..4).map(|i| Step::Stitch(Pt { x: 30 * i, y: 100 })));
    let thread = |rgb| Thread { rgb, name: None, code: None };
    let plan = StitchPlan {
        name: "T".into(),
        blocks: vec![Block { thread: thread([200, 0, 0]), steps: a }, Block { thread: thread([0, 0, 200]), steps: b }],
    };
    let o = WriteOptions { jumps_per_trim: jumps, ..Default::default() };
    // Long jumps split into records; no locks and no travel trims added to the source.
    let po = vectorcraft_stitch::PrepareOptions { locks: false, travel_trims: false };
    let max = if format == "dst" { 121 } else { 127 };
    let profile = vectorcraft_stitch::MachineProfile { max_jump: max, ..vectorcraft_stitch::GENERIC };
    let plan = vectorcraft_stitch::prepare(&plan, &profile, &po).unwrap().plan;
    match format {
        "dst" => dst::write(&plan, &o),
        _ => exp::write(&plan, &o),
    }
    .unwrap()
    .bytes
}

#[test]
fn an_opened_stitch_file_keeps_its_trims_and_gets_no_travel_trims() {
    for format in ["dst", "exp"] {
        // DST: a run of 4 jump records is a trim, so the 30 mm jumps (3 records) are plain travel.
        let jumps = if format == "dst" { 4 } else { 3 };
        let file = travelling_file(format, jumps);
        let source = read_plan_with(format, &file, jumps);
        // The explicit trim, and the ones the writer adds at the start and before the return.
        assert!(trim_count(&source) >= 1, "{format}: {source:?}");
        let mut s = session();
        let data = vectorcraft_format::base64_encode(&file);
        let open = json!({"name": format!("t.{format}"), "dataBase64": data, "stitch": {"jumpsPerTrim": jumps}});
        s.execute("document.open", &open).unwrap();
        let options = json!({"format": format, "jumpsPerTrim": jumps});
        let again = b64(&s.execute("document.export", &options).unwrap());
        let back = read_plan_with(format, &again, jumps);
        assert_eq!(trim_count(&back), trim_count(&source), "{format}: {back:?}");
        assert_eq!(needle_points(&back), needle_points(&source), "{format}");
        // `stitch.info` counts the plan's trims, not the ones the writer adds at the ends.
        let info = s.execute("stitch.info", &options).unwrap();
        assert_eq!(info["trims"].as_u64(), Some(1), "{format}: {info}");
        // Asked for, the travel trims come back: the 30 mm jumps now read as trims.
        let asked = json!({"format": format, "jumpsPerTrim": 3, "travelTrims": true});
        let more = s.execute("stitch.info", &asked).unwrap();
        assert!(more["trims"].as_u64() > info["trims"].as_u64(), "{format}: {more}");
        // Art on a fresh layer keeps the travel trims.
        s.execute("layer.new", &json!({"name": "Drawn"})).unwrap();
        s.execute("paint.setFill", &json!({"none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#00a000"})).unwrap();
        let d = 10.0 * PT_PER_MM;
        for x in [0.0, 400.0] {
            s.execute("shape.ellipse", &json!({"x": 300.0 + x, "y": 300, "width": d, "height": d})).unwrap();
        }
        let drawn = b64(&s.execute("document.export", &json!({"format": format, "jumpsPerTrim": 3})).unwrap());
        let green = read_plan_with(format, &drawn, 3);
        let last = green.blocks.last().unwrap();
        assert!(last.steps.contains(&vectorcraft_stitch::Step::Trim), "{format}: the drawn art lost its travel trim: {last:?}");
        // Off for all art when asked: a 30 mm square in manual mode has stitches that split into
        // long travel, which the rule trims by default.
        let side = 30.0 * PT_PER_MM;
        s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": side, "height": side})).unwrap();
        // (`stitch.info` counts the plan's trims; a reader also turns long jump runs into trims.)
        let trims = |s: &mut Session, extra: Value| {
            let o = with(json!({"format": format, "jumpsPerTrim": 3, "mode": "manual"}), &extra);
            s.execute("stitch.info", &o).unwrap()["trims"].as_u64().unwrap()
        };
        let (on, off) = (trims(&mut s, json!({})), trims(&mut s, json!({"travelTrims": false})));
        assert!(off < on, "{format}: {off} !< {on}");
    }
}
