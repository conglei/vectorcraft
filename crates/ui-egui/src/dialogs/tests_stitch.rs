//! Embroidery → Stitch Info and Export As → Stitch File Options, drawn headlessly.

use serde_json::json;

use super::tests_export::{Written, app, frame, kind, names, set};
use super::*;

/// An app whose one ellipse is stroked red (stitchable) and whose save dialog answers `/out/<name>`.
fn stitchable() -> (VectorcraftApp, Written) {
    let (mut app, written) = app(1);
    app.run("select.all", json!({})).unwrap();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("paint.setStroke", json!({"color": "#ff0000"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app, written)
}

#[test]
fn stitch_info_shows_the_counts_threads_and_switches_format() {
    let (mut app, _) = stitchable();
    let undo = app.session.active().unwrap().history.undo.len();
    app.run("ui.stitchInfo", json!({})).unwrap();
    assert_eq!(kind(&app), Some("stitchInfo"));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("format"), "dst");
    let info = &d.fields["__result"];
    assert!(info["stitches"].as_u64().unwrap() > 10, "{info}");
    assert_eq!(info["threads"][0]["rgb"], "#ff0000");
    let dst_records = info["records"].clone();
    // The format toggle reruns stitch.info for EXP, as the control channel sets it.
    set(&mut app, "format", json!("exp"));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("__for"), "exp");
    assert_ne!(d.fields["__result"]["records"], dst_records, "EXP has no header records");
    // Read-only: Close leaves it, and the document is as it was.
    assert!(confirm(&mut app).is_ok());
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
}

#[test]
fn stitch_info_with_nothing_stitchable_says_so_in_the_dialog() {
    // An ellipse with neither stroke nor fill: nothing is stitched, and the dialog still opens.
    let (mut app, _) = app(1);
    app.run("select.all", json!({})).unwrap();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("ui.stitchInfo", json!({})).unwrap();
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!d.fields.contains_key("__result"));
    assert!(!d.str("__error").is_empty(), "the command's error is the dialog's message");
    assert!(app.run("ui.stitchInfo", json!({"format": "svg"})).is_err());
}

#[test]
fn stitch_info_needs_a_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(app.run("ui.stitchInfo", json!({})).is_err());
    assert!(!crate::menus::enabled(&app, "ui.stitchInfo"));
}

#[test]
fn export_as_dst_shows_stitch_options_then_writes_with_them_and_remembers_them() {
    let (mut app, written) = stitchable();
    app.run("file.exportAs", json!({"format": "dst"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("stitchOptions"), "Stitch File Options follow the save panel");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("format"), d.str("machine")), ("/out/Untitled-1.dst".into(), "dst".into(), "commercial15".into()));
    assert_eq!((d.f64("stitchLength", 0.0), d.f64("minStitch", 0.0), d.f64("maxStitch", 0.0), d.f64("jumpsPerTrim", 0.0)), (2.5, 0.3, 12.1, 3.0));
    assert!(d.fields["mode"].is_null() && d.fields["locks"].is_null() && d.bool("returnToStart") && !d.bool("trimBeforeColorChange"));
    frame(&mut app);
    // Typed values reach the export: a shorter stitch makes a bigger file than the default.
    let (mut plain, plain_written) = stitchable();
    plain.run("ui.stitchOptionsDialog", json!({"path": "/out/default.dst"})).unwrap();
    confirm(&mut plain).unwrap();
    let default_size = plain_written.borrow()[0].1.len();
    set(&mut app, "stitchLength", json!(1));
    set(&mut app, "name", json!("LOGO"));
    set(&mut app, "locks", json!(false));
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(r["path"], "/out/Untitled-1.dst");
    let w = written.borrow();
    assert!(w[0].1.len() > default_size, "{} vs {default_size}", w[0].1.len());
    assert!(String::from_utf8_lossy(&w[0].1[..40]).contains("LA:LOGO"), "the design name is in the header");
    drop(w);
    // The next dialog starts from these choices, but not from this export's path.
    app.run("ui.stitchOptionsDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("stitchLength", 0.0), d.str("name"), d.fields["locks"].clone()), (1.0, "LOGO".into(), json!(false)));
    assert!(!d.fields.contains_key("path"));
    // So does Stitch Info.
    app.run("ui.stitchInfo", json!({})).unwrap();
    let with_options = app.ui.dialog.as_ref().unwrap().fields["__result"]["stitches"].as_u64().unwrap();
    app.run("ui.stitchInfo", json!({"stitchLength": 2.5})).unwrap();
    assert!(with_options > app.ui.dialog.as_ref().unwrap().fields["__result"]["stitches"].as_u64().unwrap());
}

#[test]
fn ok_passes_the_typed_values_to_the_export() {
    let (mut app, _) = stitchable();
    app.run("ui.stitchOptionsDialog", json!({"format": "exp", "path": "/out/a.exp"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.f64("maxStitch", 0.0), 12.7, "EXP's own limit is its default");
    for (k, v) in [
        ("machine", json!("generic")),
        ("mode", json!("run")),
        ("stitchLength", json!(3)),
        ("minStitch", json!(0.5)),
        ("maxStitch", json!(8)),
        ("locks", json!(true)),
        ("jumpsPerTrim", json!(4)),
        ("trimBeforeColorChange", json!(true)),
        ("start", json!("firstStitch")),
        ("returnToStart", json!(false)),
        ("name", json!("  ")),
    ] {
        set(&mut app, k, v);
    }
    frame(&mut app);
    let params = stitch_options::export_params(app.ui.dialog.as_ref().unwrap());
    assert_eq!(
        params,
        json!({"format": "exp", "path": "/out/a.exp", "machine": "generic", "mode": "run", "stitchLength": 3, "minStitch": 0.5, "maxStitch": 8,
            "locks": true, "jumpsPerTrim": 4, "trimBeforeColorChange": true, "start": "firstStitch", "returnToStart": false, "selectedOnly": false}),
        "automatic choices and a blank name are left to the engine"
    );
    confirm(&mut app).unwrap();
    assert_eq!(app.ui.stitch_options["jumpsPerTrim"], 4);
    assert!(app.ui.stitch_options.get("path").is_none() && app.ui.stitch_options.get("selectedOnly").is_none());
}

#[test]
fn bad_stitch_options_keep_the_dialog_open_and_cancel_writes_nothing() {
    let (mut app, written) = stitchable();
    app.run("ui.stitchOptionsDialog", json!({"path": "/out/a.dst"})).unwrap();
    set(&mut app, "stitchLength", json!(40));
    let e = confirm(&mut app).unwrap_err();
    assert!(e.contains("stitchLength"), "{e}");
    assert_eq!(kind(&app), Some("stitchOptions"));
    // Stitch Selected Art Only with nothing selected.
    set(&mut app, "stitchLength", json!(2.5));
    app.run("select.none", json!({})).unwrap();
    set(&mut app, "selectedOnly", json!(true));
    frame(&mut app);
    assert!(confirm(&mut app).unwrap_err().contains("select something"));
    assert!(written.borrow().is_empty());
    super::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    assert!(names(&written).is_empty(), "Cancel writes nothing");
    assert!(app.ui.stitch_options.is_null(), "and remembers nothing");
}

#[test]
fn the_stitch_options_dialog_needs_a_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(app.run("ui.stitchOptionsDialog", json!({})).is_err());
}

#[test]
fn the_embroidery_menu_reaches_the_embroidery_commands() {
    let (mut app, _) = stitchable();
    let tree = crate::menus::menu_tree();
    let titles: Vec<&str> = tree.iter().map(|(t, _)| *t).collect();
    let at = |t: &str| titles.iter().position(|x| *x == t).unwrap();
    assert_eq!(at("Embroidery"), at("Effect") + 1);
    assert_eq!(at("View"), at("Embroidery") + 1);
    let entries = crate::menus::menu_entries(&app);
    let items: Vec<(&str, Option<&str>)> =
        entries.iter().filter(|e| e.path == ["Embroidery"]).map(|e| (e.label.as_str(), e.command.as_deref())).collect();
    assert_eq!(
        items,
        [
            ("Stitch Info…", Some("ui.stitchInfo")),
            ("Stitch Preview", Some("window.panel")),
            ("Stitch Properties", Some("window.panel")),
            ("Stitch Sequence", Some("window.panel")),
            ("Sort Sequence by Colour", Some("stitch.sequence.sortByColor")),
            ("Closest Join", Some("stitch.join.closest")),
            ("Calibration Design", Some("stitch.calibration")),
            ("Photo to Stitches…", Some("ui.photoStitchDialog")),
            ("Export Stitch File…", Some("file.exportAs"))
        ]
    );
    // Calibration Design is undoable and adds the test piece.
    let before = app.session.active().unwrap().history.undo.len();
    crate::menus::invoke(&mut app, "stitch.calibration", json!({}));
    assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1);
    // Export Stitch File… opens Export As on DST.
    crate::menus::invoke(&mut app, "file.exportAs", json!({"format": "dst"}));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("format"), "dst");
}

#[test]
fn the_machine_and_stitch_type_choices_reach_the_export() {
    let (mut app, _) = stitchable();
    app.run("ui.stitchOptionsDialog", json!({"path": "/out/a.dst"})).unwrap();
    for (k, v) in [("machine", json!("bai_mirror")), ("fillType", json!("none")), ("strokeType", json!("satin"))] {
        set(&mut app, k, v);
    }
    frame(&mut app);
    let params = stitch_options::export_params(app.ui.dialog.as_ref().unwrap());
    assert_eq!(params["machine"], "bai_mirror");
    assert_eq!((params["fill"].clone(), params["stroke"].clone()), (json!({"type": "none"}), json!({"type": "satin"})));
    assert!(params.get("fillType").is_none() && params.get("strokeType").is_none());
    confirm(&mut app).unwrap();
    // The defaults are left to the engine.
    app.run("ui.stitchOptionsDialog", json!({"fillType": "tatami", "strokeType": "run"})).unwrap();
    let params = stitch_options::export_params(app.ui.dialog.as_ref().unwrap());
    assert!(params.get("fill").is_none() && params.get("stroke").is_none());
}

#[test]
fn stitch_info_switches_to_dsb_and_back() {
    let (mut app, _) = stitchable();
    app.run("ui.stitchInfo", json!({"format": "dsb"})).unwrap();
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("format"), d.str("__for")), ("dsb".into(), "dsb".into()));
    let dsb_records = d.fields["__result"]["records"].clone();
    // DSB's longest stitch is 12.7 mm, like EXP's.
    assert_eq!(stitch_options::max_stitch("dsb"), 12.7);
    set(&mut app, "format", json!("dst"));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("__for"), "dst");
    set(&mut app, "format", json!("dsb"));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("__for"), d.fields["__result"]["records"].clone()), ("dsb".into(), dsb_records));
    assert!(app.run("ui.stitchInfo", json!({"format": "pes"})).is_err());
}

#[test]
fn export_as_dsb_shows_stitch_options_with_its_own_limit_then_writes_a_dsb_file() {
    let (mut app, written) = stitchable();
    app.run("file.exportAs", json!({"format": "dsb"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("stitchOptions"), "Stitch File Options follow the save panel for DSB too");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("format")), ("/out/Untitled-1.dsb".into(), "dsb".into()));
    assert_eq!(d.f64("maxStitch", 0.0), 12.7, "DSB's own limit is its default, as EXP's");
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["path"], "/out/Untitled-1.dsb");
    let w = written.borrow();
    assert!(w[0].1.starts_with(b"LA:"), "a DSB file has the 512-byte header");
    assert_eq!(w[0].1.len() % 3, 512 % 3, "512 header bytes, then three-byte records");
    assert_eq!(w[0].1.get(w[0].1.len() - 3), Some(&0xF8), "ends with the DSB end record");
}

#[test]
fn photo_to_stitches_runs_the_command_and_opens_the_design() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    // Off no document: the dialog needs none.
    assert!(crate::menus::enabled(&app, "ui.photoStitchDialog"));
    let dir = std::env::temp_dir().join(format!("vc-photostitch-dialog-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let photo = dir.join("fur.png");
    app.run("photo.syntheticFur", json!({"path": photo, "width": 160, "height": 120})).unwrap();
    crate::menus::invoke(&mut app, "ui.photoStitchDialog", json!({}));
    assert_eq!(kind(&app), Some("photoStitch"));
    frame(&mut app);
    // No photo yet: the error stays in the dialog.
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some("photoStitch"));
    assert!(app.ui.dialog.as_ref().unwrap().str("__error").contains("photo"));
    frame(&mut app);
    // A bad file: the command's error, still in the dialog.
    set(&mut app, "file", json!(dir.join("missing.png")));
    assert!(confirm(&mut app).is_err());
    assert!(!app.ui.dialog.as_ref().unwrap().str("__error").is_empty());
    // The fields reach the command; OK opens the design and closes the dialog.
    for (k, v) in [
        ("file", json!(photo)),
        ("provider", json!("none")),
        ("widthMm", json!(50)),
        ("fill", json!("meander")),
        ("spacing", json!(1.5)),
        ("seed", json!(3)),
    ] {
        set(&mut app, k, v);
    }
    frame(&mut app);
    let params = photo_stitch::command_params(app.ui.dialog.as_ref().unwrap());
    assert_eq!((params["fill"].as_str(), params["provider"].as_str(), params["widthMm"].as_f64()), (Some("meander"), Some("none"), Some(50.0)));
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(r["fill"], "meander");
    assert_eq!(app.session.documents().len(), 1);
    assert!(r["stitches"].as_u64().unwrap() > 100, "{r}");
    // The preference's provider is left to the command.
    photo_stitch::open(&mut app, &json!({}));
    assert!(photo_stitch::command_params(app.ui.dialog.as_ref().unwrap()).get("provider").is_none());
}
