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
            ("Stitches on the Canvas", Some("view.stitches")),
            ("Hoop on the Canvas", Some("view.hoop")),
            ("Stitch Info…", Some("ui.stitchInfo")),
            ("Stitch Preview", Some("window.panel")),
            ("Stitch Properties", Some("window.panel")),
            ("Stitch Sequence", Some("window.panel")),
            ("Sort Sequence by Colour", Some("stitch.sequence.sortByColor")),
            ("Closest Join", Some("stitch.join.closest")),
            ("Entry and Exit Tool", Some("tool.select")),
            ("Remove Overlaps", Some("stitch.overlap.remove")),
            ("Route Within Colours", Some("stitch.route")),
            ("Calibration Design", Some("stitch.calibration")),
            ("Photo to Stitches…", Some("ui.photoStitchDialog")),
            ("Convert Type to Stitch Objects", Some("text.toStitches")),
            ("Auto-Digitize Tool", Some("ui.autoDigitizeTool")),
            ("Auto-Digitize Selection", Some("stitch.autoDigitize")),
            ("Auto-Digitize Image…", Some("ui.autoDigitizeImageDialog")),
            ("Center Design in Hoop", Some("stitch.hoop.center")),
            ("Production Worksheet…", Some("ui.stitchWorksheet")),
            ("Export for Production…", Some("ui.stitchProduction")),
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

#[test]
fn stitch_info_shows_estimates_needles_the_hoop_and_the_design_check() {
    let (mut app, _) = stitchable();
    app.run("ui.stitchInfo", json!({"machine": "bai_mirror"})).unwrap();
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    let info = &d.fields["__result"];
    assert!(info["runTimeS"].as_f64().unwrap() > 0.0, "{info}");
    assert!(info["bobbinMm"].as_f64().unwrap() > 0.0, "{info}");
    assert_eq!(info["threads"][0]["needle"], 1);
    assert!(d.fields["__check"]["findings"].as_array().unwrap().iter().any(|f| f["code"] == "hoop"), "{}", d.fields["__check"]);
    assert_eq!(d.fields["__hoops"]["hoops"].as_array().unwrap().len(), 10);
    // Choosing a hoop is the engine's stitch.hoop.set, one undo step.
    let undo = app.session.active().unwrap().history.undo.len();
    app.run("stitch.hoop.set", json!({"id": "square-100"})).unwrap();
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    app.run("ui.stitchInfo", json!({"machine": "bai_mirror"})).unwrap();
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["__hoops"]["chosen"]["id"], "square-100");
}

#[test]
fn production_worksheet_and_export_dialogs_run_their_commands() {
    let (mut app, _) = stitchable();
    let dir = std::env::temp_dir().join(format!("vc-ui-production-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sheet = dir.join("sheet.pdf").to_string_lossy().to_string();
    app.run("ui.stitchWorksheet", json!({"path": sheet})).unwrap();
    assert_eq!(kind(&app), Some("stitchWorksheet"));
    frame(&mut app);
    assert!(confirm(&mut app).is_ok());
    assert!(app.ui.dialog.is_none());
    assert!(std::fs::read(&sheet).unwrap().starts_with(b"%PDF"));
    let out = dir.join("folder").to_string_lossy().to_string();
    app.run("ui.stitchProduction", json!({"dir": out, "name": "ring", "machine": "bai_mirror"})).unwrap();
    assert_eq!(kind(&app), Some("stitchProduction"));
    frame(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(d.bool("dst") && d.bool("dsb") && !d.bool("exp") && d.bool("worksheet"));
    assert!(confirm(&mut app).is_ok());
    for f in ["ring.dst", "ring.dsb", "ring-worksheet.pdf"] {
        assert!(dir.join("folder").join(f).is_file(), "{f}");
    }
    // An empty folder keeps the dialog open.
    app.run("ui.stitchProduction", json!({"dir": ""})).unwrap();
    assert!(confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some());
    let _ = std::fs::remove_dir_all(&dir);
    let mut none = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(none.run("ui.stitchWorksheet", json!({})).is_err());
    assert!(none.run("ui.stitchProduction", json!({})).is_err());
    assert!(!crate::menus::enabled(&none, "ui.stitchProduction"));
}

#[test]
fn auto_digitize_image_runs_the_command_and_shows_what_it_made() {
    let (mut app, _) = app(1);
    // No image selected: off, and an error.
    assert!(!crate::menus::enabled(&app, "ui.autoDigitizeImageDialog"));
    assert!(app.run("ui.autoDigitizeImageDialog", json!({})).is_err());
    let dir = std::env::temp_dir().join(format!("vc-autodigitize-dialog-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let photo = dir.join("fur.png");
    app.run("photo.syntheticFur", json!({"path": photo, "width": 120, "height": 90})).unwrap();
    app.run("file.place", json!({"path": photo, "link": false})).unwrap();
    assert!(crate::menus::enabled(&app, "ui.autoDigitizeImageDialog"));
    crate::menus::invoke(&mut app, "ui.autoDigitizeImageDialog", json!({"colors": 3}));
    assert_eq!(kind(&app), Some("autoDigitizeImage"));
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert!(r["regions"].as_u64().unwrap() >= 1, "{r}");
    // The dialog stays to say what it made; OK again closes it.
    assert!(app.ui.dialog.as_ref().unwrap().str("__result").contains("Fills:"));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), None);
    // The tool: a menu item chooses it.
    crate::menus::invoke(&mut app, "ui.autoDigitizeTool", json!({}));
    assert_eq!(app.session.tool_id(), "autoDigitize");
}

/// An app with the file services a desktop host gives: files read from disk, and an open dialog
/// answering `picked` (the `FilePick`s it was asked for are kept in the second value).
fn app_with_files(picked: Option<String>) -> (VectorcraftApp, std::rc::Rc<std::cell::RefCell<Vec<crate::FilePick>>>) {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let log = asked.clone();
    let services = crate::Services {
        read_head: Some(Box::new(|p: &str, max: usize| {
            use std::io::Read;
            let mut out = vec![];
            std::fs::File::open(p).and_then(|f| f.take(max as u64).read_to_end(&mut out)).map_err(|e| e.to_string())?;
            Ok(out)
        })),
        pick_open: Some(Box::new(move |pick: &crate::FilePick| {
            log.borrow_mut().push(pick.clone());
            picked.clone()
        })),
        ..Default::default()
    };
    (VectorcraftApp::new(vectorcraft_engine::Session::new(), services), asked)
}

/// A PNG of `w` × `h` pixels in a folder of its own.
fn png_file(name: &str, w: u32, h: u32) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-photostitch-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("photo.png");
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 255 / w.max(1)) as u8, (y * 255 / h.max(1)) as u8, 90, 255]));
    img.save(&path).unwrap();
    path
}

/// Egui frames of the dialog layer that keep their context, so a texture made in one is there in the next.
fn frames(app: &mut VectorcraftApp, ctx: &egui::Context, n: usize) {
    for _ in 0..n {
        let mut out = ctx.run_ui(Default::default(), |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    }
}

fn thumbnail_kept(ctx: &egui::Context) -> bool {
    ctx.data(|m| m.get_temp::<(String, egui::TextureHandle)>(egui::Id::unique("photo-stitch-thumbnail"))).is_some()
}

/// The photo field is typed or filled; once it names a photo the dialog knows its pixel size and
/// holds a thumbnail, and a path that isn't a photo says so instead.
#[test]
fn photo_to_stitches_shows_the_size_and_a_thumbnail_of_the_photo() {
    let photo = png_file("thumb", 400, 300);
    let (mut app, _) = app_with_files(None);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    photo_stitch::open(&mut app, &json!({}));
    frames(&mut app, &ctx, 2);
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("__preview").is_none(), "no photo, nothing to show");
    set(&mut app, "file", json!(photo));
    frames(&mut app, &ctx, 2);
    let preview = app.ui.dialog.as_ref().unwrap().fields["__preview"].clone();
    assert_eq!((preview["width"].as_u64(), preview["height"].as_u64()), (Some(400), Some(300)), "{preview}");
    assert!(thumbnail_kept(&ctx), "the thumbnail is kept");
    // Not read again while the path stays.
    set(&mut app, "__preview", json!({"path": photo, "width": 1, "height": 1}));
    frames(&mut app, &ctx, 1);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__preview"]["width"], 1);
    // A file that is no photo, and one that is not there, say so and show no thumbnail.
    let text = photo.with_file_name("notes.png");
    std::fs::write(&text, b"just words").unwrap();
    for bad in [text, photo.with_file_name("missing.png")] {
        set(&mut app, "file", json!(bad));
        frames(&mut app, &ctx, 2);
        let preview = &app.ui.dialog.as_ref().unwrap().fields["__preview"];
        assert!(preview.get("width").is_none() && !preview["note"].as_str().unwrap().is_empty(), "{preview}");
        assert!(!thumbnail_kept(&ctx));
    }
    // Cleared: nothing.
    set(&mut app, "file", json!(""));
    frames(&mut app, &ctx, 1);
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("__preview").is_none());
    let _ = std::fs::remove_dir_all(photo.parent().unwrap());
}

/// A photo of more pixels than the thumbnail decodes is sized from its header only.
#[test]
fn photo_to_stitches_sizes_a_huge_photo_without_decoding_it() {
    // A PNG header that claims 20000 × 20000 pixels (400 megapixels) and has no pixels.
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in bytes {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(20_000u32.to_be_bytes());
    ihdr.extend(20_000u32.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    for chunk in [ihdr, b"IDAT".to_vec(), b"IEND".to_vec()] {
        bytes.extend((chunk.len() as u32 - 4).to_be_bytes());
        bytes.extend(&chunk);
        bytes.extend(crc(&chunk).to_be_bytes());
    }
    let (size, image) = photo_stitch::thumbnail(&bytes, 192).unwrap();
    assert_eq!(size, (20_000, 20_000));
    assert!(image.is_none());
}

/// Choose… asks the host's open dialog for a photo type, and the file picked fills the field, in
/// the folder of the last one the next time.
#[test]
fn photo_to_stitches_choose_fills_the_field_and_remembers_the_folder() {
    let photo = png_file("choose", 64, 48);
    let (mut app, asked) = app_with_files(Some(photo.to_string_lossy().to_string()));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    photo_stitch::open(&mut app, &json!({}));
    frames(&mut app, &ctx, 1);
    let mut d = app.ui.dialog.clone().unwrap();
    photo_stitch::choose(&mut app, &mut d);
    app.ui.dialog = Some(d);
    let pick = asked.borrow()[0].clone();
    assert_eq!(pick.filters.len(), 1);
    let exts = &pick.filters[0].1;
    for e in ["png", "jpg", "jpeg", "heic", "tif", "tiff", "webp"] {
        assert!(exts.iter().any(|x| x == e), "{e} in {exts:?}");
    }
    assert_eq!(pick.folder, None, "nothing chosen before");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("file"), photo.to_string_lossy());
    assert_eq!(app.ui.photo_folder.as_deref(), photo.parent().and_then(|p| p.to_str()));
    frames(&mut app, &ctx, 2);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__preview"]["width"], 64);
    // The folder is where the picker starts the next time.
    app.ui.dialog = None;
    photo_stitch::open(&mut app, &json!({}));
    let mut d = app.ui.dialog.clone().unwrap();
    photo_stitch::choose(&mut app, &mut d);
    assert_eq!(asked.borrow()[1].folder.as_deref(), photo.parent().and_then(|p| p.to_str()));
    let _ = std::fs::remove_dir_all(photo.parent().unwrap());
}

/// A cancelled picker leaves the field as it was.
#[test]
fn photo_to_stitches_choose_cancelled_changes_nothing() {
    let (mut app, asked) = app_with_files(None);
    photo_stitch::open(&mut app, &json!({"file": "/some/where/a.jpg"}));
    let mut d = app.ui.dialog.clone().unwrap();
    photo_stitch::choose(&mut app, &mut d);
    assert_eq!(d.str("file"), "/some/where/a.jpg");
    assert_eq!(asked.borrow()[0].folder.as_deref(), Some("/some/where"), "the picker starts where the photo is");
    assert_eq!(app.ui.photo_folder, None);
}

/// A photo dropped on the window while Photo to Stitches is open goes to its field and opens no
/// document; a drop with no photo in it changes nothing and says so.
#[test]
fn a_photo_dropped_on_photo_to_stitches_fills_the_field() {
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }
    let handle = |p: &std::path::Path| std::sync::Arc::new(Dropped(p.into())) as egui::DroppedFileHandle;
    let photo = png_file("drop", 32, 32);
    let svg = photo.with_file_name("art.svg");
    std::fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="5" height="5"/>"#).unwrap();
    let (mut app, _) = app_with_files(None);
    photo_stitch::open(&mut app, &json!({}));
    app.open_dropped_files(vec![handle(&svg)], None, false);
    assert!(app.session.documents().is_empty(), "no document opens");
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("file"), "");
    assert!(app.ui.status.contains("Drop a PNG"), "{}", app.ui.status);
    app.open_dropped_files(vec![handle(&svg), handle(&photo)], None, false);
    assert!(app.session.documents().is_empty(), "no document opens");
    assert_eq!(kind(&app), Some("photoStitch"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("file"), photo.to_string_lossy());
    assert_eq!(app.ui.photo_folder.as_deref(), photo.parent().and_then(|p| p.to_str()));
    // The dialog closed, a drop opens documents as before.
    app.ui.dialog = None;
    app.open_dropped_files(vec![handle(&svg)], None, false);
    assert_eq!(app.session.documents().len(), 1);
    let _ = std::fs::remove_dir_all(photo.parent().unwrap());
}
