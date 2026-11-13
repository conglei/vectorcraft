//! Stitch Properties panel (Window › Stitch Properties, Embroidery › Stitch Properties): the stitch
//! type of the selected paths, compound paths and groups, and the parameters of what they sew.
//! It reads `stitch.props` and acts through `stitch.setType`, `stitch.setParams` and
//! `stitch.clear`. Automatic objects follow the document's defaults, the Filled Paths and Stroked
//! Paths of the Stitch File Options, which `stitch.props` is given as its `options`.

use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{LetterTrims, StitchParam, StitchType, Underlay};

use super::selection_len;
use crate::VectorcraftApp;
use crate::widgets;

pub const ID: &str = "stitchProps";
const LABEL_W: f32 = 104.0;
const FIELD_W: f32 = 72.0;

/// The document's defaults as `stitch.props` options: the Stitch File Options used last
/// (`fillType`, `strokeType`, `stitchLength`, `mode`), as an export takes them.
pub(crate) fn default_options(app: &VectorcraftApp) -> Value {
    let Some(o) = app.ui.stitch_options.as_object() else { return json!({}) };
    let mut out = Map::new();
    if let Some(v) = o.get("stitchLength").filter(|v| v.is_number()) {
        out.insert("stitchLength".into(), v.clone());
    }
    if let Some(v) = o.get("mode").filter(|v| v.is_string()) {
        out.insert("mode".into(), v.clone());
    }
    if let Some(v) = o.get("fillType").filter(|v| v.is_string()) {
        out.insert("fill".into(), json!({ "type": v }));
    }
    if let Some(v) = o.get("strokeType").filter(|v| v.is_string()) {
        out.insert("stroke".into(), json!({ "type": v }));
    }
    Value::Object(out)
}

/// What the panel shows for the selection: `stitch.props`'s objects, recomputed only when the
/// document, the selection or the defaults change.
fn objects(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Value> {
    type Key = (u64, u64, Vec<vectorcraft_doc::NodeId>, String);
    let Some(st) = app.session.active() else { return vec![] };
    if st.selection.is_empty() {
        return vec![];
    }
    let options = default_options(app);
    let key: Key = (st.uid, st.revision, st.selection.objects.clone(), options.to_string());
    let id = egui::Id::unique("stitch-props-state");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<(Key, Vec<Value>)>(id))
        && k == key
    {
        return v;
    }
    // A selection of other kinds alone is an error: nothing to show.
    let r = app.session.execute("stitch.props", &json!({ "options": options }));
    let v = r.ok().and_then(|r| r["objects"].as_array().cloned()).unwrap_or_default();
    ctx.data_mut(|d| d.insert_temp(id, (key, v.clone())));
    v
}

/// The one value every object has at `f`, if they share it.
fn shared<'a>(objects: &'a [Value], f: impl Fn(&'a Value) -> &'a Value) -> Option<&'a Value> {
    let first = f(objects.first()?);
    objects.iter().all(|o| f(o) == first).then_some(first)
}

/// The label of the selection's own stitch type: "Mixed" when the objects differ.
fn type_label(objects: &[Value]) -> &'static str {
    match shared(objects, |o| &o["own"]["type"]) {
        None => tl!("Mixed"),
        Some(v) => v.as_str().and_then(StitchType::parse).unwrap_or_default().label(),
    }
}

/// The numbers shown for what the objects sew: the fill's for tatami, the satin's, the running
/// stitch length.
fn shown_params(objects: &[Value]) -> Vec<StitchParam> {
    let mut sews: Vec<StitchType> = vec![];
    for o in objects {
        let types: Vec<StitchType> = o["sews"].as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(StitchType::parse).collect();
        // A group or layer shows the numbers of what the objects in it sew (`sews` lists that).
        for t in types {
            if !sews.contains(&t) {
                sews.push(t);
            }
        }
    }
    // Compensation is stored but not used yet: commands set it, the panel leaves it out.
    StitchParam::ALL
        .into_iter()
        .filter(|p| *p != StitchParam::Compensation && p.used_by().iter().any(|t| *t != StitchType::Auto && sews.contains(t)))
        .collect()
}

/// The field label of number `p`.
fn param_label(p: StitchParam) -> &'static str {
    match p {
        StitchParam::FillAngle => tl!("Fill Angle:"),
        StitchParam::FillSpacing => tl!("Row Spacing:"),
        StitchParam::FillStitchLength => tl!("Fill Stitch Length:"),
        StitchParam::Stagger => tl!("Stagger:"),
        StitchParam::SatinSpacing => tl!("Satin Spacing:"),
        StitchParam::SatinWidth => tl!("Satin Width:"),
        StitchParam::RunLength => tl!("Stitch Length:"),
        StitchParam::Compensation => tl!("Compensation:"),
        StitchParam::MinColumn => tl!("Min. Column:"),
        StitchParam::RunUnder => tl!("Run Under:"),
    }
}

/// The unit suffix of number `p` in its field.
fn suffix(p: StitchParam) -> &'static str {
    match p.unit() {
        "mm" => " mm",
        "degrees" => "°",
        _ => "",
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let objects = objects(app, ui.ctx());
    if objects.is_empty() {
        let msg = if selection_len(app) == 0 { tl!("No Selection") } else { tl!("Select paths, compound paths or groups to set how they are sewn.") };
        widgets::dim_label(ui, msg);
        caption(ui);
        return;
    }
    let labels: Vec<&str> = StitchType::ALL.iter().map(|t| t.label()).collect();
    widgets::label_row(ui, tl!("Stitch Type:"), LABEL_W, |ui| {
        if let Some(t) = widgets::dropdown(ui, "stitch-props-type", type_label(&objects), &labels, (ui.available_width() - 8.0).clamp(80.0, 200.0))
            .and_then(|i| StitchType::ALL.get(i))
        {
            app.run("stitch.setType", json!({ "type": t.id() })).ok();
        }
    });
    let sews = match shared(&objects, |o| &o["sews"]) {
        Some(v) => {
            let names: Vec<&str> =
                v.as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(StitchType::parse).map(StitchType::label).collect();
            if names.is_empty() { tl!("Not sewn").to_string() } else { names.join(", ") }
        }
        None => tl!("Mixed").to_string(),
    };
    widgets::label_row(ui, tl!("Sewn as:"), LABEL_W, |ui| {
        widgets::dim_label(ui, &sews);
    });
    if objects.iter().any(|o| o["opened"] == true) {
        widgets::dim_label(ui, tl!("On an opened stitch file's layer: sewn as its needle points."));
    }
    let params = shown_params(&objects);
    if !params.is_empty() {
        widgets::divider(ui);
    }
    for p in params {
        let value = shared(&objects, |o| &o["params"][p.key()]).and_then(Value::as_f64);
        let (lo, hi) = p.range();
        widgets::label_row(ui, param_label(p), LABEL_W, |ui| {
            let decimals = if p == StitchParam::FillAngle { 0 } else { 2 };
            if let Some(v) = widgets::mixed_field(ui, ("stitch-props", p.key()), value, suffix(p), decimals, FIELD_W) {
                app.run("stitch.setParams", json!({ "params": { p.key(): v.clamp(lo, hi) } })).ok();
            }
            if p == StitchParam::SatinWidth && value.is_none() && shared(&objects, |o| &o["params"][p.key()]).is_some() {
                widgets::dim_label(ui, tl!("stroke weight"));
            }
        });
    }
    underlay(app, ui, &objects);
    letter_trims(app, ui, &objects);
    sequencing(app, ui, &objects);
    ui.add_space(6.0);
    let own = objects.iter().any(|o| o["own"].as_object().is_some_and(|m| !m.is_empty()));
    ui.horizontal(|ui| {
        let r = ui.add_enabled_ui(own, |ui| widgets::flat_button(ui, tl!("Reset to Automatic"), 140.0)).inner;
        if r.on_hover_text(tl!("Clear the selection's stitch type and parameters: they follow the groups holding them and the document's defaults"))
            .clicked()
        {
            app.run("stitch.clear", json!({})).ok();
        }
    });
    caption(ui);
}

/// The Underlay dropdown, for what sews satin columns (two-rail satin and lettering).
fn underlay(app: &mut VectorcraftApp, ui: &mut Ui, objects: &[Value]) {
    let satin = objects.iter().any(|o| o["sews"].as_array().is_some_and(|v| v.iter().any(|t| t == "lettering" || t == "satin")));
    if !satin {
        return;
    }
    let current = match shared(objects, |o| &o["params"]["underlay"]) {
        Some(v) => v.as_str().and_then(Underlay::parse).unwrap_or_default().label(),
        None => tl!("Mixed"),
    };
    let labels: Vec<&str> = Underlay::ALL.iter().map(|u| u.label()).collect();
    // Automatic: a center run under narrow columns, an edge walk from 3 mm, a zigzag too from 6 mm.
    widgets::label_row(ui, tl!("Underlay:"), LABEL_W, |ui| {
        if let Some(u) = widgets::dropdown(ui, "stitch-props-underlay", current, &labels, 160.0).and_then(|i| Underlay::ALL.get(i)) {
            app.run("stitch.setParams", json!({ "params": { "underlay": u.id() } })).ok();
        }
    });
}

/// Lettering's Trims dropdown, for type sewn as lettering.
fn letter_trims(app: &mut VectorcraftApp, ui: &mut Ui, objects: &[Value]) {
    let lettering = objects.iter().any(|o| o["sews"].as_array().is_some_and(|v| v.iter().any(|t| t == "lettering")));
    if !lettering {
        return;
    }
    let current = match shared(objects, |o| &o["params"]["letterTrims"]) {
        Some(v) => v.as_str().and_then(LetterTrims::parse).unwrap_or_default().label(),
        None => tl!("Mixed"),
    };
    let labels: Vec<&str> = LetterTrims::ALL.iter().map(|t| t.label()).collect();
    widgets::label_row(ui, tl!("Trims:"), LABEL_W, |ui| {
        if let Some(t) = widgets::dropdown(ui, "stitch-props-trims", current, &labels, 120.0).and_then(|i| LetterTrims::ALL.get(i)) {
            app.run("stitch.setParams", json!({ "params": { "letterTrims": t.id() } })).ok();
        }
    });
}

/// Embroidery › Remove Overlaps and Route Within Colors, when the selection has them: a line each
/// with a Clear button (`stitch.overlap.clear`, `stitch.route {maxTravelMm: 0}`).
fn sequencing(app: &mut VectorcraftApp, ui: &mut Ui, objects: &[Value]) {
    let ids = |key: &str| -> Vec<u64> { objects.iter().filter(|o| o[key].is_number()).filter_map(|o| o["id"].as_u64()).collect() };
    let mm = |key: &str| -> String {
        match shared(objects, |o| &o[key]).and_then(Value::as_f64) {
            Some(v) => format!("{v:.1} mm"),
            None => tl!("mixed").to_string(),
        }
    };
    for (key, label, tip, cmd, params) in [
        ("overlapMm", tl!("Overlaps removed"), tl!("Sew the fill whole again, under the fills sewn after it"), "stitch.overlap.clear", json!({})),
        ("routeMm", tl!("Routed travel"), tl!("Enter it by a trim again, not by travel"), "stitch.route", json!({"maxTravelMm": 0})),
    ] {
        let listed = ids(key);
        if listed.is_empty() {
            continue;
        }
        let text = format!("{label} ({})", mm(key));
        ui.horizontal(|ui| {
            widgets::dim_label(ui, &text);
            if widgets::flat_button(ui, tl!("Clear"), 56.0).on_hover_text(tip).clicked() {
                let mut p = params.clone();
                p["ids"] = json!(listed);
                app.run(cmd, p).ok();
            }
        });
    }
}

/// Where the defaults come from.
fn caption(ui: &mut Ui) {
    ui.add_space(4.0);
    widgets::dim_label(
        ui,
        tl!(
            "Automatic objects follow the document defaults: Filled Paths and Stroked Paths in File › Export Stitch File options. Stitches are made again from each object's outline when it is moved or scaled."
        ),
    );
}

/// The panel (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let own = objects(app, ui.ctx()).iter().any(|o| o["own"].as_object().is_some_and(|m| !m.is_empty()));
    if widgets::menu_item(ui, tl!("Reset to Automatic"), own, false) {
        app.run("stitch.clear", json!({})).ok();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    /// Run the panel and its menu for one headless frame: the texts drawn.
    fn frame(app: &mut VectorcraftApp) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    fn app_with_rects() -> (VectorcraftApp, u64, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
        app.run("paint.setStroke", json!({"color": "#ff0000"})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        let b = app.run("shape.rectangle", json!({"x": 100, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        (app, a, b)
    }

    #[test]
    fn removed_overlaps_show_a_line_with_clear() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap();
        app.run("select.none", json!({})).unwrap();
        app.run("shape.rectangle", json!({"x": 30, "y": 30, "width": 60, "height": 60})).unwrap();
        app.run("stitch.overlap.remove", json!({})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Overlaps removed (0.7 mm)") && t.iter().any(|s| s == "Clear"), "{t:?}");
        app.run("stitch.overlap.clear", json!({"ids": [a]})).unwrap();
        assert!(!frame(&mut app).iter().any(|s| s.starts_with("Overlaps removed")));
    }

    /// STI-38: satin shows the underlay choice; tatami doesn't.
    #[test]
    fn satin_shows_its_underlay() {
        let (mut app, a, _) = app_with_rects();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        assert!(!frame(&mut app).iter().any(|s| s == "Underlay:"));
        app.run("stitch.setType", json!({"type": "satin"})).unwrap();
        app.run("stitch.setParams", json!({"params": {"underlay": "edge"}})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Underlay:") && t.iter().any(|s| s == "Edge walk"), "{t:?}");
    }

    #[test]
    fn the_panel_shows_the_type_and_the_parameters_of_what_is_sewn() {
        let (mut app, a, _) = app_with_rects();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Automatic"), "{t:?}");
        assert!(t.iter().any(|s| s == "Tatami fill, Running stitch"), "{t:?}");
        assert!(t.iter().any(|s| s == "Row Spacing:") && t.iter().any(|s| s == "Stitch Length:"), "{t:?}");
        assert!(!t.iter().any(|s| s == "Satin Width:"), "no satin sewn: {t:?}");
        // Satin: its fields, and the stroke weight standing for an unset width.
        app.run("stitch.setType", json!({"type": "satin"})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Satin column"), "{t:?}");
        assert!(t.iter().any(|s| s == "Satin Width:") && t.iter().any(|s| s == "stroke weight"), "{t:?}");
        assert!(!t.iter().any(|s| s == "Row Spacing:"), "{t:?}");
        // Not sewn: no fields.
        app.run("stitch.setType", json!({"type": "none"})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Not sewn"), "{t:?}");
        assert!(!t.iter().any(|s| s.ends_with("Spacing:")), "{t:?}");
    }

    /// A group shows what the objects in it sew, with their fields; an opened stitch file's group
    /// shows that its runs are sewn point by point, and no fill fields (STI-33).
    #[test]
    fn a_group_shows_what_its_objects_sew() {
        let (mut app, a, b) = app_with_rects();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("object.group", json!({})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Tatami fill, Running stitch"), "{t:?}");
        assert!(!t.iter().any(|s| s == "Not sewn"), "{t:?}");
        assert!(t.iter().any(|s| s == "Row Spacing:") && !t.iter().any(|s| s == "Satin Width:"), "no satin in it: {t:?}");
        // The same design as a stitch file, opened: a click selects a group of runs.
        let file = app.run("document.export", json!({"format": "dst"})).unwrap();
        app.run("document.open", json!({"name": "a.dst", "dataBase64": file["dataBase64"]})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Manual (one point per anchor)"), "{t:?}");
        assert!(!t.iter().any(|s| s == "Not sewn"), "{t:?}");
        assert!(t.iter().any(|s| s.starts_with("On an opened stitch file")), "{t:?}");
        assert!(!t.iter().any(|s| s == "Row Spacing:" || s == "Fill Angle:"), "nothing to fill: {t:?}");
    }

    #[test]
    fn a_selection_that_differs_shows_mixed() {
        let (mut app, a, b) = app_with_rects();
        app.run("stitch.setType", json!({"type": "run", "ids": [a]})).unwrap();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().filter(|s| *s == "Mixed").count() >= 2, "the type and what is sewn: {t:?}");
        // Setting a type from the panel's command reaches both, as one undo step.
        app.run("stitch.setType", json!({"type": "tatami"})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Tatami fill") && !t.iter().any(|s| s == "Mixed"), "{t:?}");
    }

    #[test]
    fn nothing_selected_and_other_kinds_explain_themselves() {
        let (mut app, _, _) = app_with_rects();
        app.run("select.none", json!({})).unwrap();
        assert!(frame(&mut app).iter().any(|s| s == "No Selection"));
        app.run("text.create", json!({"x": 10, "y": 200, "text": "Hi"})).unwrap();
        assert!(selection_len(&app) > 0);
        // Type is sewn as lettering, with its trims.
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Lettering (type)") && t.iter().any(|s| s == "Per word"), "{t:?}");
    }

    #[test]
    fn the_defaults_come_from_the_stitch_file_options() {
        let (mut app, a, _) = app_with_rects();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.ui.stitch_options = json!({"fillType": "none", "strokeType": "satin", "stitchLength": 3});
        assert_eq!(default_options(&app), json!({"fill": {"type": "none"}, "stroke": {"type": "satin"}, "stitchLength": 3}));
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "Satin column"), "the stroke is sewn as satin, the fill not: {t:?}");
    }

    #[test]
    fn window_and_embroidery_menus_open_the_panel() {
        let (mut app, _, _) = app_with_rects();
        assert!(crate::state::ICON_PANELS.iter().any(|(id, label, _)| *id == ID && *label == "Stitch Properties"));
        let entries = crate::menus::menu_entries(&app);
        for path in [["Window"], ["Embroidery"]] {
            assert!(entries.iter().any(|e| e.path == path && e.label == "Stitch Properties"), "{path:?}");
        }
        app.run("window.panel", json!({"panel": ID})).unwrap();
        assert!(crate::panel_docking::showing(&app.ui, ID));
    }
}
