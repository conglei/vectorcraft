//! The hoop and fabric of an embroidery design, as New Document, Document Setup, Properties and
//! Stitch Info show them: the hoop dropdown over `stitch.hoops.list`, a custom hoop's size and
//! shape, and the fabric colour swatch.
//!
//! A hoop is held as `stitch.setup` takes it: `null` (none), a library hoop as
//! `stitch.hoops.list` lists it (`{id, name, widthMm, heightMm, shape, cap}`), or a custom one
//! (`{id: null, name?, widthMm, heightMm, shape}`).

use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::widgets;

/// The size a hoop switched to Custom starts at when there was none, mm.
const CUSTOM_MM: f64 = 100.0;
/// The hoop New Document offers in an embroidery app: a common mid-size frame.
pub(crate) const NEW_DESIGN_HOOP: &str = "rect-130x180";
/// Millimetres to points.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// What the hoop dropdown picked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HoopPick {
    /// None (or Automatic, as the caller names it).
    None,
    /// The library hoop at this index of the list.
    Library(usize),
    /// Custom: a size of the person's own.
    Custom,
}

/// The library hoops (`stitch.hoops.list`'s), empty when the list can't be had.
pub(crate) fn library(app: &mut crate::VectorcraftApp) -> Vec<Value> {
    app.session.execute("stitch.hoops.list", &json!({})).ok().and_then(|r| r.get("hoops").and_then(Value::as_array).cloned()).unwrap_or_default()
}

/// Whether `hoop` is a custom one (an object without a library id).
fn is_custom(hoop: &Value) -> bool {
    hoop.is_object() && !hoop.get("id").is_some_and(Value::is_string)
}

/// The hoop dropdown: `none` first, then the library `list`, then Custom when `custom` offers it;
/// a custom `chosen` hoop the dropdown can't make shows as its own last entry. → what was picked,
/// when it differs from `chosen`.
pub(crate) fn hoop_dropdown(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    (list, chosen): (&[Value], &Value),
    none: &str,
    custom: bool,
    width: f32,
) -> Option<HoopPick> {
    let mut labels: Vec<String> = vec![none.to_string()];
    labels.extend(list.iter().map(|h| h.get("name").and_then(Value::as_str).unwrap_or("").to_string()));
    let custom_at = labels.len();
    if custom {
        labels.push(tl!("Custom").to_string());
    }
    let current = match chosen.get("id").and_then(Value::as_str) {
        Some(id) => list.iter().position(|h| h.get("id").and_then(Value::as_str) == Some(id)).map_or(0, |i| i + 1),
        None if is_custom(chosen) && custom => custom_at,
        None if is_custom(chosen) => {
            labels.push(chosen.get("name").and_then(Value::as_str).unwrap_or("").to_string());
            labels.len() - 1
        }
        None => 0,
    };
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    let i = widgets::dropdown(ui, id, names.get(current).copied().unwrap_or(""), &names, width).filter(|i| *i != current)?;
    match i {
        0 => Some(HoopPick::None),
        i if i <= list.len() => Some(HoopPick::Library(i - 1)),
        i if custom && i == custom_at => Some(HoopPick::Custom),
        // The custom hoop itself: nothing to change.
        _ => None,
    }
}

/// The hoop `pick` makes from `list`, starting a custom one from `current`'s size and shape.
pub(crate) fn picked(pick: &HoopPick, list: &[Value], current: &Value) -> Value {
    match pick {
        HoopPick::None => Value::Null,
        HoopPick::Library(i) => list.get(*i).cloned().unwrap_or(Value::Null),
        HoopPick::Custom => {
            let side = |k: &str| current.get(k).and_then(Value::as_f64).unwrap_or(CUSTOM_MM);
            let shape = current.get("shape").and_then(Value::as_str).unwrap_or("rectangle");
            json!({"id": null, "widthMm": side("widthMm"), "heightMm": side("heightMm"), "shape": shape})
        }
    }
}

/// A hoop's size in points (width, height), when it has one.
pub(crate) fn size_pt(hoop: &Value) -> Option<(f64, f64)> {
    let w = hoop.get("widthMm").and_then(Value::as_f64)?;
    let h = hoop.get("heightMm").and_then(Value::as_f64)?;
    Some((w * PT_PER_MM, h * PT_PER_MM))
}

/// The hoop section of a dialog: the dropdown and, for a custom hoop, its width, height and shape.
/// `hoop` is edited in place. → whether it changed.
pub(crate) fn hoop_fields(ui: &mut egui::Ui, id: &str, list: &[Value], hoop: &mut Value, (label, width): (f32, f32)) -> bool {
    let mut changed = false;
    widgets::label_row(ui, tl!("Hoop:"), label, |ui| {
        if let Some(p) = hoop_dropdown(ui, (id, "hoop"), (list, hoop), tl!("None"), true, width) {
            *hoop = picked(&p, list, hoop);
            changed = true;
        }
    });
    if is_custom(hoop) {
        widgets::label_row(ui, "", label, |ui| {
            for (key, name) in [("widthMm", tl!("W:")), ("heightMm", tl!("H:"))] {
                ui.label(egui::RichText::new(name).color(Tokens::get(ui.ctx()).text_dim));
                let v = hoop.get(key).and_then(Value::as_f64).unwrap_or(CUSTOM_MM);
                if let Some(n) = widgets::plain_field_named(ui, (id, key), Some(name), v, "mm", 1, 70.0) {
                    hoop[key] = json!(n.clamp(10.0, 2000.0));
                    changed = true;
                }
                ui.add_space(4.0);
            }
        });
        widgets::label_row(ui, tl!("Shape:"), label, |ui| {
            let shapes = [tl!("Rectangle"), tl!("Round")];
            let round = hoop.get("shape").and_then(Value::as_str) == Some("round");
            if let Some(i) = widgets::dropdown(ui, (id, "shape"), shapes[usize::from(round)], &shapes, 120.0) {
                hoop["shape"] = json!(if i == 1 { "round" } else { "rectangle" });
                changed = true;
            }
        });
    }
    changed
}

/// The colour `#rrggbb` names, if it is one.
pub(crate) fn rgb_of(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#')?;
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    (h.len() == 6).then_some(())?;
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `rgb` as `#rrggbb`.
pub(crate) fn hex_of([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The fabric swatch with its hex value, editing `hex` (`#rrggbb`) in place. → whether it changed.
pub(crate) fn fabric_field(ui: &mut egui::Ui, id: &str, hex: &mut String, label: f32) -> bool {
    let mut changed = false;
    widgets::label_row(ui, tl!("Fabric:"), label, |ui| {
        let mut rgb = rgb_of(hex).unwrap_or(vectorcraft_doc::stitch_design::DEFAULT_FABRIC);
        if ui
            .push_id((id, "fabric"), |ui| ui.color_edit_button_srgb(&mut rgb))
            .inner
            .on_hover_text(tl!("The fabric the stitches are previewed on"))
            .changed()
        {
            *hex = hex_of(rgb);
            changed = true;
        }
        ui.label(egui::RichText::new(hex.as_str()).monospace().color(Tokens::get(ui.ctx()).text_dim));
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_make_the_hoops_stitch_setup_takes() {
        let list = vec![json!({"id": "square-100", "name": "Square", "widthMm": 100.0, "heightMm": 100.0, "shape": "rectangle"})];
        assert_eq!(picked(&HoopPick::None, &list, &Value::Null), Value::Null);
        assert_eq!(picked(&HoopPick::Library(0), &list, &Value::Null)["id"], "square-100");
        let custom = picked(&HoopPick::Custom, &list, &list[0]);
        assert_eq!(custom, json!({"id": null, "widthMm": 100.0, "heightMm": 100.0, "shape": "rectangle"}));
        assert!(is_custom(&custom) && !is_custom(&list[0]) && !is_custom(&Value::Null));
        let (w, h) = size_pt(&custom).unwrap();
        assert!((w - 283.464_566_9).abs() < 1e-6 && (h - w).abs() < 1e-9);
        assert_eq!(rgb_of("#0a0B0c"), Some([10, 11, 12]));
        assert_eq!((rgb_of("0a0b0c"), rgb_of("#0a0b0"), rgb_of("#ééé")), (None, None, None));
        assert_eq!(hex_of([10, 11, 12]), "#0a0b0c");
    }
}
