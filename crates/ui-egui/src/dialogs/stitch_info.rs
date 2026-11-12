//! Embroidery → Stitch Info…: what a DST, DSB or EXP export of the document would write, without
//! writing a file: the stitches, jumps, trims and colour changes, the records in the file, the
//! size of the design in mm, one row per thread (its colour, stitches and length in mm) and the
//! warnings. Read-only: the choice of DST, DSB or EXP reruns `stitch.info`, with the options the last
//! stitch export used. A command error (nothing stitchable, a bad option) shows as the dialog's
//! message.
//!
//! It also shows the estimates (run time, top and bobbin thread), a needle per thread, the hoop
//! (a dropdown that runs `stitch.hoop.set`) and a Design Check section listing what `stitch.check`
//! finds.
//!
//! Fields: `format` (`dst` | `dsb` | `exp`), and the dialog's own `__params` (the `stitch.info` options),
//! `__for` (the format the result is for), `__result` (what `stitch.info` returned), `__check` (what
//! `stitch.check` returned), `__hoops` (`stitch.hoops.list`) and `__error`.

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

/// The format a dialog is for: `exp` or `dsb`, else `dst`.
fn format_of(d: &Dialog) -> &'static str {
    stitch_options::format_id(&d.str("format"))
}

/// Open Stitch Info on the active document: `p` is `{format?: dst (default) | dsb | exp, …stitch.info
/// options}`, over the options the last stitch export used.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let format = match p.get("format").and_then(Value::as_str) {
        None | Some("dst") => "dst",
        Some("dsb") => "dsb",
        Some("exp") => "exp",
        Some(_) => return Err("format must be dst, dsb or exp".into()),
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
    // The checker and the hoop list are extras: when they fail, their sections are left out.
    let check = app.session.execute("stitch.check", &params).unwrap_or(Value::Null);
    d.fields.insert("__check".into(), check);
    let machine = json!({"machine": params.get("machine").cloned().unwrap_or(json!("commercial15"))});
    let hoops = app.session.execute("stitch.hoops.list", &machine).unwrap_or(Value::Null);
    d.fields.insert("__hoops".into(), hoops);
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
        for (id, label) in [("dst", "DST"), ("dsb", "DSB"), ("exp", "EXP")] {
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
            (tl!("Run Time"), run_time(&info)),
            (tl!("Top Thread"), metres(&info, "topMm")),
            (tl!("Bobbin (estimate)"), metres(&info, "bobbinMm")),
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
            egui::Grid::new("stitch-threads-grid").num_columns(5).spacing([18.0, 4.0]).show(ui, |ui| {
                for h in [tl!("Color"), "", tl!("Needle"), tl!("Stitches"), tl!("Length")] {
                    ui.label(egui::RichText::new(h).color(t.text_dim).size(11.5));
                }
                ui.end_row();
                for th in threads {
                    let hex = th.get("rgb").and_then(Value::as_str).unwrap_or("#000000");
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, color(hex));
                    ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                    ui.label(egui::RichText::new(hex).color(t.text).size(12.5).monospace());
                    ui.label(egui::RichText::new(count(th, "needle")).color(t.text).size(12.5));
                    ui.label(egui::RichText::new(count(th, "stitches")).color(t.text).size(12.5));
                    let length = mm(th, "lengthMm");
                    let length = crate::i18n::fmt(tl!("{length} mm"), &[("length", length.as_str())]);
                    ui.label(egui::RichText::new(length).color(t.text).size(12.5));
                    ui.end_row();
                }
            });
        });
    }
    hoop_row(app, ui, d);
    design_check(ui, d, &t);
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

/// `v[key]` mm as metres with one decimal.
fn metres(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_f64).map_or_else(|| "-".into(), |n| format!("{:.1} m", n / 1000.0))
}

/// The estimated run time at the speed `stitch.info` used.
fn run_time(v: &Value) -> String {
    let (Some(secs), Some(spm)) = (v.get("runTimeS").and_then(Value::as_f64), v.get("spm").and_then(Value::as_f64)) else { return "-".into() };
    // Finite (JSON numbers are) and capped, so the cast is exact.
    let s = secs.clamp(0.0, 1e9) as u64;
    let time = if s >= 3600 { format!("{} h {:02} min", s / 3600, (s % 3600) / 60) } else { format!("{} min {:02} s", s / 60, s % 60) };
    format!("{time} ({spm:.0} spm)")
}

/// The hoop: a dropdown of the machine's hoops (Automatic: none chosen, the smallest that fits).
fn hoop_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let Some(list) = d.fields.get("__hoops").and_then(|h| h.get("hoops")).and_then(Value::as_array).cloned() else { return };
    let chosen = d.fields.get("__hoops").and_then(|h| h.get("chosen")).cloned().unwrap_or(Value::Null);
    let mut labels: Vec<String> = vec![tl!("Automatic (smallest that fits)").to_string()];
    labels.extend(list.iter().map(|h| h.get("name").and_then(Value::as_str).unwrap_or("").to_string()));
    let current = match chosen.get("id").and_then(Value::as_str) {
        Some(id) => list.iter().position(|h| h.get("id").and_then(Value::as_str) == Some(id)).map_or(0, |i| i + 1),
        None if chosen.is_object() => {
            labels.push(chosen.get("name").and_then(Value::as_str).unwrap_or("").to_string());
            labels.len() - 1
        }
        None => 0,
    };
    let names: Vec<&str> = labels.iter().map(String::as_str).collect();
    ui.add_space(10.0);
    let mut picked = None;
    widgets::label_row(ui, tl!("Hoop:"), 70.0, |ui| {
        picked = widgets::dropdown(ui, "stitch-info-hoop", names.get(current).copied().unwrap_or(""), &names, 260.0);
    });
    let Some(i) = picked.filter(|i| *i != current) else { return };
    let params = match i.checked_sub(1).and_then(|k| list.get(k)) {
        Some(h) => json!({"id": h.get("id").cloned().unwrap_or(Value::Null)}),
        None if i == 0 => json!({"clear": true}),
        // The custom hoop itself: nothing to change.
        None => return,
    };
    if let Err(e) = app.run("stitch.hoop.set", params) {
        app.status(e);
    }
    refresh(app, d);
}

/// Design Check: what `stitch.check` found, errors first.
fn design_check(ui: &mut egui::Ui, d: &Dialog, t: &Tokens) {
    let Some(findings) = d.fields.get("__check").and_then(|c| c.get("findings")).and_then(Value::as_array) else { return };
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Design Check"));
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("stitch-check").max_height(LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
        for f in findings {
            let (mark, colour) = match f.get("level").and_then(Value::as_str) {
                Some("error") => ("●", t.error),
                Some("warning") => ("▲", t.warning),
                _ => ("○", t.text_dim),
            };
            let message = f.get("message").and_then(Value::as_str).unwrap_or("");
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(mark).color(colour).size(11.0));
                ui.add(egui::Label::new(egui::RichText::new(crate::i18n::msg(message)).color(t.text).size(12.0)).wrap());
            });
        }
    });
}
