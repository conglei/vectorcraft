//! Embroidery → Stitch Info…: what a DST or EXP export of the document would write, without
//! writing a file: the stitches, jumps, trims and colour changes, the records in the file, the
//! size of the design in mm, one row per thread (its colour, stitches and length in mm) and the
//! warnings. Read-only: the choice of DST or EXP reruns `stitch.info`, with the options the last
//! stitch export used. A command error (nothing stitchable, a bad option) shows as the dialog's
//! message.
//!
//! Fields: `format` (`dst` | `exp`), and the dialog's own `__params` (the `stitch.info` options),
//! `__for` (the format the result is for), `__result` (what `stitch.info` returned) and `__error`.

use serde_json::{Value, json};

use super::{DialogSpec, stitch_options};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "stitchInfo";

/// What the dialog is for (its OK is Close: `ok` is none).
pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Stitch Info").into(), body, ok: None, min_width: 420.0, max_width: Some(480.0), ..DialogSpec::FORM };

/// The most the thread list shows before it scrolls.
const LIST_HEIGHT: f32 = 170.0;

/// The format a dialog is for: `exp`, else `dst`.
fn format_of(d: &Dialog) -> &'static str {
    if d.str("format") == "exp" { "exp" } else { "dst" }
}

/// Open Stitch Info on the active document: `p` is `{format?: dst (default) | exp, …stitch.info
/// options}`, over the options the last stitch export used.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let format = match p.get("format").and_then(Value::as_str) {
        None | Some("dst") => "dst",
        Some("exp") => "exp",
        Some(_) => return Err("format must be dst or exp".into()),
    };
    let mut d = Dialog::new(KIND, json!({ "format": format, "__params": p }));
    refresh(app, &mut d);
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// Run `stitch.info` for the dialog's format and keep what it returns (or says).
fn refresh(app: &mut VectorcraftApp, d: &mut Dialog) {
    let format = format_of(d);
    let given = d.fields.get("__params").cloned().unwrap_or(Value::Null);
    let options = stitch_options::fields(app, format, &given);
    let mut params = stitch_options::export_params(&Dialog::new(stitch_options::KIND, Value::Object(options)));
    params["format"] = json!(format);
    match app.session.execute("stitch.info", &params) {
        Ok(v) => {
            d.fields.insert("__result".into(), v);
            d.fields.remove("__error");
        }
        Err(e) => {
            d.fields.remove("__result");
            d.fields.insert("__error".into(), json!(e.to_string()));
        }
    }
    d.fields.insert("__for".into(), json!(format));
}

/// The colour `#rrggbb` names.
fn color(hex: &str) -> egui::Color32 {
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0);
    egui::Color32::from_rgb(byte(1), byte(3), byte(5))
}

/// A whole number from `v[key]`, as text.
fn count(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_u64).map_or_else(|| "-".into(), |n| n.to_string())
}

/// `v[key]` in mm with one decimal.
fn mm(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_f64).map_or_else(|| "-".into(), |n| format!("{n:.1}"))
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let before = format_of(d);
    widgets::label_row(ui, tl!("Format:"), 70.0, |ui| {
        for (id, label) in [("dst", "DST"), ("exp", "EXP")] {
            if widgets::radio(ui, label, before == id, true) {
                d.fields.insert("format".into(), json!(id));
            }
            ui.add_space(12.0);
        }
    });
    if d.str("__for") != format_of(d) {
        refresh(app, d);
    }
    ui.add_space(10.0);
    if let Some(e) = d.fields.get("__error").and_then(Value::as_str) {
        ui.add(egui::Label::new(egui::RichText::new(crate::i18n::msg(e)).color(t.error).size(12.5)).wrap());
        return false;
    }
    let Some(info) = d.fields.get("__result").cloned() else { return false };
    egui::Grid::new("stitch-info").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        let (w, h) = (mm(&info, "widthMm"), mm(&info, "heightMm"));
        let size = crate::i18n::fmt(tl!("{width} × {height} mm"), &[("width", w.as_str()), ("height", h.as_str())]);
        let rows = [
            (tl!("Stitches"), count(&info, "stitches")),
            (tl!("Jumps"), count(&info, "jumps")),
            (tl!("Trims"), count(&info, "trims")),
            (tl!("Color Changes"), count(&info, "colorChanges")),
            (tl!("Records in the File"), count(&info, "records")),
            (tl!("Size"), size),
        ];
        for (label, value) in rows {
            ui.label(egui::RichText::new(label).color(t.text_dim).size(12.5));
            ui.label(egui::RichText::new(value).color(t.text).size(12.5));
            ui.end_row();
        }
    });
    let threads = info.get("threads").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    if !threads.is_empty() {
        ui.add_space(10.0);
        widgets::subheader(ui, tl!("Threads"));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("stitch-threads").max_height(LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("stitch-threads-grid").num_columns(4).spacing([18.0, 4.0]).show(ui, |ui| {
                for h in [tl!("Color"), "", tl!("Stitches"), tl!("Length")] {
                    ui.label(egui::RichText::new(h).color(t.text_dim).size(11.5));
                }
                ui.end_row();
                for th in threads {
                    let hex = th.get("rgb").and_then(Value::as_str).unwrap_or("#000000");
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, color(hex));
                    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    ui.label(egui::RichText::new(hex).color(t.text).size(12.5).monospace());
                    ui.label(egui::RichText::new(count(th, "stitches")).color(t.text).size(12.5));
                    let length = mm(th, "lengthMm");
                    let length = crate::i18n::fmt(tl!("{length} mm"), &[("length", length.as_str())]);
                    ui.label(egui::RichText::new(length).color(t.text).size(12.5));
                    ui.end_row();
                }
            });
        });
    }
    let warnings = info.get("warnings").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    if !warnings.is_empty() {
        ui.add_space(10.0);
        widgets::subheader(ui, tl!("Warnings"));
        ui.add_space(2.0);
        for w in warnings.iter().filter_map(Value::as_str) {
            ui.add(egui::Label::new(egui::RichText::new(crate::i18n::msg(w)).color(t.warning).size(12.0)).wrap());
        }
    }
    false
}
