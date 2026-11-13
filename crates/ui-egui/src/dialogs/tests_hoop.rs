//! The hoop and fabric of an embroidery design in New Document, Document Setup and Properties,
//! drawn headlessly: shown in an embroidery app (or for a design with a hoop), never otherwise.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// Every text drawn in one frame of `f`, on a 1400 × 1000 screen.
fn texts(app: &mut VectorcraftApp, mut f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<String> {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    let mut out = vec![];
    // Two frames: the first only measures new windows.
    for _ in 0..2 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1000.0))), ..Default::default() };
        let mut frame = ctx.run_ui(raw, |ui| f(app, ui));
        frame.textures_delta.clear();
        out.clear();
        frame.shapes.iter().for_each(|c| walk(&c.shape, &mut out));
    }
    out
}

fn dialogs(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::dialogs::show(app, ui.ctx());
}

fn properties(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::panels::properties::show(app, ui);
}

fn embroidery_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.open_in_stitch_view();
    app
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().and_then(|d| d.fields.get(k).cloned()).unwrap_or_default()
}

#[test]
fn new_document_offers_a_hoop_in_an_embroidery_app_and_creates_the_design_in_it() {
    let mut app = embroidery_app();
    app.run("file.newDialog", json!({})).unwrap();
    let shown = texts(&mut app, dialogs);
    assert!(shown.iter().any(|t| t == "Hoop:") && shown.iter().any(|t| t == "Fabric:"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Rectangle 130 × 180 mm"), "the default hoop: {shown:?}");
    // The size is the hoop's.
    let pt = 72.0 / 25.4;
    assert!((field(&app, "width").as_f64().unwrap() - 130.0 * pt).abs() < 1e-6);
    assert!((field(&app, "height").as_f64().unwrap() - 180.0 * pt).abs() < 1e-6);
    assert_eq!(field(&app, "units"), "Millimeters");
    // A custom round hoop shows its size and shape fields and sets the size.
    let custom = json!({"id": null, "widthMm": 150.0, "heightMm": 150.0, "shape": "round"});
    app.ui.dialog.as_mut().unwrap().fields.insert("hoop".into(), custom.clone());
    app.ui.dialog.as_mut().unwrap().fields.insert("fabric".into(), json!("#203040"));
    let shown = texts(&mut app, dialogs);
    assert!(shown.iter().any(|t| t == "Custom") && shown.iter().any(|t| t == "Round") && shown.iter().any(|t| t == "Shape:"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "#203040"), "{shown:?}");
    // Create: the design is born with its hoop and fabric, its artboard the hoop's size.
    app.ui.dialog.as_mut().unwrap().fields.remove("width");
    app.ui.dialog.as_mut().unwrap().fields.remove("height");
    crate::dialogs::confirm(&mut app).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.stitch.hoop().map(|h| (h.width_mm, h.shape.id())), Some((150.0, "round")));
    assert_eq!(doc.stitch.fabric, Some([0x20, 0x30, 0x40]));
    assert!((doc.artboards[0].rect.width() - 150.0 * pt).abs() < 1e-6);
    assert!(app.session.active().unwrap().history.undo.is_empty());
}

#[test]
fn vectorcrafts_new_document_and_document_setup_have_no_hoop() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.newDialog", json!({})).unwrap();
    let shown = texts(&mut app, dialogs);
    assert!(!shown.iter().any(|t| t == "Hoop:" || t == "Fabric:"), "{shown:?}");
    assert_eq!(field(&app, "hoop"), Value::Null);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.session.active().unwrap().doc.stitch.hoop, None);
    app.run("file.documentSetup", json!({})).unwrap();
    let shown = texts(&mut app, dialogs);
    assert!(shown.iter().any(|t| t == "Transparency") && !shown.iter().any(|t| t == "Embroidery"), "{shown:?}");
    // The Properties panel has no Hoop row either.
    app.ui.dialog = None;
    let shown = texts(&mut app, properties);
    assert!(shown.iter().any(|t| t == "Document Setup") && !shown.iter().any(|t| t == "Hoop" || t == "Fabric"), "{shown:?}");
    // A design with a hoop shows it anywhere.
    app.run("stitch.setup", json!({"hoop": "square-100"})).unwrap();
    let shown = texts(&mut app, properties);
    assert!(shown.iter().any(|t| t == "Hoop") && shown.iter().any(|t| t == "Square 100 × 100 mm"), "{shown:?}");
}

#[test]
fn document_setup_sets_the_hoop_fabric_and_artboard_in_one_step() {
    let mut app = embroidery_app();
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("file.documentSetup", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("tab".into(), json!("Embroidery"));
    let shown = texts(&mut app, dialogs);
    for s in ["Embroidery", "Hoop:", "Fabric:", "Resize the Artboard to the Hoop", "#ece8de"] {
        assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
    }
    let undo = app.session.active().unwrap().history.undo.len();
    let mut e = field(&app, "embroidery");
    e["hoop"] = json!("square-200");
    e["fabric"] = json!("#334455");
    e["resizeArtboard"] = json!(true);
    app.ui.dialog.as_mut().unwrap().fields.insert("embroidery".into(), e);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1, "one step");
    assert_eq!(st.doc.stitch.hoop().and_then(|h| h.id.clone()).as_deref(), Some("square-200"));
    assert!((st.doc.artboards[0].rect.width() - 200.0 * 72.0 / 25.4).abs() < 1e-6);
}

#[test]
fn properties_shows_the_hoop_row_and_fabric_in_an_embroidery_app() {
    let mut app = embroidery_app();
    app.run("file.new", json!({"hoop": "round-150", "fabric": "#102030"})).unwrap();
    let shown = texts(&mut app, properties);
    for s in ["Hoop", "Round 150 mm", "Fabric", "#102030"] {
        assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
    }
    // With no hoop yet the row offers None.
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let shown = texts(&mut app, properties);
    assert!(shown.iter().any(|t| t == "Hoop") && shown.iter().any(|t| t == "None"), "{shown:?}");
}
