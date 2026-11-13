//! Stitch Sequence panel (Window › Stitch Sequence, Embroidery › Stitch Sequence): the order the
//! objects are sewn in, one row per object (its thread colours, name, stitch types and stitch
//! count), with Move Up, Move Down, Sort by Colour and Closest Join. It reads
//! `stitch.sequence.list` and acts through `stitch.sequence.move`, `stitch.sequence.sortByColor`,
//! `stitch.join.closest` and `select.set`; a row click selects its object, and the selected
//! objects' rows are highlighted.

use egui::{Rect, Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, StitchType};

use super::stitch_props::default_options;
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const ID: &str = "stitchSequence";
const ROW_H: f32 = 22.0;

/// `stitch.sequence.list` for the document, recomputed only when the document or the defaults
/// change. `None`: no document, or nothing to sew.
fn listed(app: &mut VectorcraftApp, ctx: &egui::Context) -> Option<Value> {
    type Key = (u64, u64, String);
    let st = app.session.active()?;
    let options = default_options(app);
    let key: Key = (st.uid, st.revision, options.to_string());
    let id = egui::Id::unique("stitch-sequence-state");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<(Key, Option<Value>)>(id))
        && k == key
    {
        return v;
    }
    let v = app.session.execute("stitch.sequence.list", &json!({ "options": options })).ok();
    ctx.data_mut(|d| d.insert_temp(id, (key, v.clone())));
    v
}

/// A `#rrggbb` colour.
fn colour(v: &Value) -> egui::Color32 {
    let h = v.as_str().and_then(|s| s.strip_prefix('#')).filter(|h| h.len() == 6 && h.is_ascii()).unwrap_or("000000");
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok()).unwrap_or(0);
    egui::Color32::from_rgb(byte(0), byte(2), byte(4))
}

/// The stitch types an object sews, as the row says them.
fn sews_text(o: &Value) -> String {
    let names: Vec<&str> = o["sews"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(StitchType::parse)
        .map(|t| match t {
            StitchType::Tatami => tl!("Tatami"),
            StitchType::Satin => tl!("Satin"),
            StitchType::Manual => tl!("Manual"),
            StitchType::Lettering => tl!("Lettering"),
            _ => tl!("Run"),
        })
        .collect();
    names.join(" + ")
}

/// One row per object; the selected objects' rows are highlighted. The object clicked, if any.
fn rows(ui: &mut Ui, t: &Tokens, objects: &[Value], selected: &[NodeId]) -> Option<u64> {
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("stitch-sequence-rows").max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, o) in objects.iter().enumerate() {
            let id = o["id"].as_u64().unwrap_or(0);
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            if selected.contains(&NodeId(id)) {
                ui.painter().rect_filled(row, 3.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(row, 3.0, t.hover);
            }
            let font = egui::FontId::proportional(12.0);
            ui.painter().text(row.left_center() + vec2(4.0, 0.0), egui::Align2::LEFT_CENTER, format!("{}", i + 1), font.clone(), t.text_dim);
            // A swatch per thread the object sews in, side by side.
            let mut x = 26.0;
            for c in o["colors"].as_array().into_iter().flatten() {
                let swatch = Rect::from_min_size(row.left_center() + vec2(x, -7.0), vec2(14.0, 14.0));
                ui.painter().rect_filled(swatch, 2.0, colour(c));
                ui.painter().rect_stroke(swatch, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                x += 16.0;
            }
            let name = o["name"].as_str().unwrap_or_default();
            let pin = if o["pinned"] == true { tl!(" (fixed entry)") } else { "" };
            ui.painter().text(row.left_center() + vec2(x + 4.0, 0.0), egui::Align2::LEFT_CENTER, format!("{name}{pin}"), font.clone(), t.text);
            let stitches = o["stitches"].as_u64().unwrap_or(0);
            let detail = format!("{} · {}", sews_text(o), crate::i18n::tn(stitches, "{n} stitch", "{n} stitches"));
            ui.painter().text(row.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, detail, font, t.text_dim);
            if resp.clicked() {
                clicked = Some(id);
            }
        }
    });
    clicked
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(list) = listed(app, ui.ctx()) else {
        widgets::dim_label(ui, tl!("Nothing to sew: draw stroked or filled paths."));
        return;
    };
    let objects: Vec<Value> = list["objects"].as_array().cloned().unwrap_or_default();
    if objects.is_empty() {
        widgets::dim_label(ui, tl!("Nothing to sew: draw stroked or filled paths."));
        return;
    }
    let t = Tokens::get(ui.ctx());
    let selected: Vec<NodeId> = app.session.active().map(|st| st.selection.objects.clone()).unwrap_or_default();
    let changes = list["colorChanges"].as_u64().unwrap_or(0);
    let trims = list["trims"].as_u64().unwrap_or(0);
    widgets::dim_label(
        ui,
        &format!(
            "{} · {} · {}",
            crate::i18n::tn(objects.len() as u64, "{n} object", "{n} objects"),
            crate::i18n::tn(changes, "{n} colour change", "{n} colour changes"),
            crate::i18n::tn(trims, "{n} trim", "{n} trims"),
        ),
    );
    if let Some(id) = rows(ui, &t, &objects, &selected) {
        app.run("select.set", json!({ "ids": [id] })).ok();
    }
    // Move Up and Move Down act on the one selected object in the order.
    let index = match selected.as_slice() {
        [one] => objects.iter().position(|o| o["id"].as_u64() == Some(one.0)),
        _ => None,
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let up = index.is_some_and(|i| i > 0);
        if ui.add_enabled_ui(up, |ui| widgets::flat_button(ui, tl!("Move Up"), 80.0)).inner.clicked()
            && let (Some(i), Some(one)) = (index, selected.first())
        {
            app.run("stitch.sequence.move", json!({ "id": one.0, "to": i.saturating_sub(1) })).ok();
        }
        let down = index.is_some_and(|i| i + 1 < objects.len());
        if ui.add_enabled_ui(down, |ui| widgets::flat_button(ui, tl!("Move Down"), 80.0)).inner.clicked()
            && let (Some(i), Some(one)) = (index, selected.first())
        {
            app.run("stitch.sequence.move", json!({ "id": one.0, "to": i + 1 })).ok();
        }
    });
    ui.horizontal(|ui| {
        let options = default_options(app);
        if widgets::flat_button(ui, tl!("Sort by Colour"), 110.0)
            .on_hover_text(tl!("Group the order by thread colour, keeping the order within each colour"))
            .clicked()
        {
            app.run("stitch.sequence.sortByColor", json!({ "options": options })).ok();
        }
        if widgets::flat_button(ui, tl!("Closest Join"), 110.0)
            .on_hover_text(tl!("Enter each object nearest where the one before it was left: fewer trims, less travel"))
            .clicked()
        {
            app.run("stitch.join.closest", json!({ "options": options })).ok();
        }
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    /// Run the panel for one headless frame: the texts drawn and the panel's buttons' rects by
    /// label.
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

    fn app_with_lines() -> (VectorcraftApp, Vec<u64>) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("paint.setFill", json!({"none": true})).unwrap();
        let mut ids = vec![];
        for (i, c) in ["#ff0000", "#0000ff", "#ff0000"].iter().enumerate() {
            app.run("select.none", json!({})).unwrap();
            app.run("paint.setStroke", json!({"color": c})).unwrap();
            let x = 20.0 + 40.0 * i as f64;
            ids.push(app.run("shape.line", json!({"x1": x, "y1": 0, "x2": x, "y2": 100})).unwrap()["id"].as_u64().unwrap());
        }
        app.run("select.none", json!({})).unwrap();
        (app, ids)
    }

    fn order(app: &mut VectorcraftApp) -> Vec<u64> {
        let l = app.run("stitch.sequence.list", json!({})).unwrap();
        l["objects"].as_array().unwrap().iter().map(|o| o["id"].as_u64().unwrap()).collect()
    }

    #[test]
    fn the_panel_lists_the_order_with_its_colour_changes() {
        let (mut app, _) = app_with_lines();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "3 objects · 2 colour changes · 0 trims"), "{t:?}");
        assert_eq!(t.iter().filter(|s| s.as_str() == "<Line>" || s.as_str() == "<Path>").count(), 3, "{t:?}");
        assert!(t.iter().any(|s| s.starts_with("Run · ")), "{t:?}");
        for b in ["Move Up", "Move Down", "Sort by Colour", "Closest Join"] {
            assert!(t.iter().any(|s| s == b), "{b}: {t:?}");
        }
        // Sort by Colour, as the button runs it: one colour change left, and a trim between the
        // two red lines (both drawn downwards).
        app.run("stitch.sequence.sortByColor", json!({})).unwrap();
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "3 objects · 1 colour change · 1 trim"), "{t:?}");
    }

    #[test]
    fn moving_the_selected_object_changes_the_order() {
        let (mut app, ids) = app_with_lines();
        app.run("select.set", json!({"ids": [ids[2]]})).unwrap();
        frame(&mut app);
        app.run("stitch.sequence.move", json!({"id": ids[2], "to": 1})).unwrap();
        assert_eq!(order(&mut app), vec![ids[0], ids[2], ids[1]]);
        let t = frame(&mut app);
        assert!(t.iter().any(|s| s == "3 objects · 1 colour change · 1 trim"), "{t:?}");
    }

    #[test]
    fn an_empty_document_explains_itself() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        assert!(frame(&mut app).iter().any(|s| s.starts_with("Nothing to sew")));
    }

    #[test]
    fn window_and_embroidery_menus_open_the_panel() {
        let (mut app, _) = app_with_lines();
        assert!(crate::state::ICON_PANELS.iter().any(|(id, label, _)| *id == ID && *label == "Stitch Sequence"));
        let entries = crate::menus::menu_entries(&app);
        for path in [["Window"], ["Embroidery"]] {
            assert!(entries.iter().any(|e| e.path == path && e.label == "Stitch Sequence"), "{path:?}");
        }
        app.run("window.panel", json!({"panel": ID})).unwrap();
        assert!(crate::panel_docking::showing(&app.ui, ID));
    }
}
