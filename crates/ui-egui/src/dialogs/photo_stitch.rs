//! Embroidery → Photo to Stitches…: turn a photo into a stitch design. The dialog only holds the
//! params of `photo.toStitches` (the photo's file, provider, thread colours, width, fill, spacing,
//! feather, the eye and nose detail pass and the seed); OK runs the command, which opens the design
//! as a new document, and closes. A command error stays in the dialog as its message.
//!
//! Fields: `file`, `provider` (null: the `photoProvider` preference), `colors`, `widthMm`, `fill`,
//! `spacing`, `feather`, `features`, `seed`, and `__error` (the last error).
//! Spec: `docs/superpowers/specs/2026-11-12-embroidery-photo-second-pass.md`.

use serde_json::{Map, Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "photoStitch";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Photo to Stitches").into(),
    body,
    confirm,
    ok: Some("OK"),
    min_width: 440.0,
    max_width: Some(500.0),
    inputs: |_| &["file", "provider", "colors", "widthMm", "fill", "spacing", "feather", "features", "seed"],
    ..DialogSpec::FORM
};

const LABEL: f32 = 150.0;
const NUMBER_W: f32 = 70.0;

/// Who prepares the photo: (`provider`, label); null is the preference.
const PROVIDERS: [(Option<&str>, &str); 4] = [
    (None, "Preference (Settings)"),
    (Some("none"), "None: the photo as it is"),
    (Some("nomodel"), "No Model (on this computer)"),
    (Some("gemini"), "Gemini (sends the photo to Google)"),
];
/// The fills: (`fill`, label).
const FILLS: [(&str, &str); 4] = [
    ("field", "Colour: rows follow the fur"),
    ("tatami", "Colour: editable tatami regions"),
    ("rows", "One colour: rows by tone"),
    ("meander", "One colour: meander by tone"),
];

/// The fields a dialog starts from: the command's defaults.
pub(crate) fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [
        ("file", json!("")),
        ("provider", Value::Null),
        ("colors", json!(8)),
        ("widthMm", json!(100)),
        ("fill", json!("field")),
        ("spacing", json!(0.4)),
        ("feather", json!(0)),
        ("features", json!(true)),
        ("seed", json!(1)),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// Open Photo to Stitches with `params` (any of the fields) over the defaults.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let mut fields = defaults();
    if let Some(o) = params.as_object() {
        fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

/// The `photo.toStitches` params the fields stand for: the preference's provider is left out.
pub(crate) fn command_params(d: &Dialog) -> Value {
    let mut p = Map::new();
    p.insert("file".into(), json!(d.str("file").trim()));
    if let Some(provider) = d.fields.get("provider").and_then(Value::as_str) {
        p.insert("provider".into(), json!(provider));
    }
    for key in ["colors", "widthMm", "spacing", "feather", "seed"] {
        if let Some(v) = d.fields.get(key).and_then(Value::as_f64) {
            p.insert(key.into(), json!(v));
        }
    }
    p.insert("fill".into(), json!(d.str("fill")));
    p.insert("features".into(), json!(d.bool("features")));
    Value::Object(p)
}

/// A dropdown row: `label`, then the dropdown showing `current` among `labels`. → the index picked.
fn pick(ui: &mut egui::Ui, id: &str, label: &str, current: usize, labels: &[&str], width: f32) -> Option<usize> {
    let mut chosen = None;
    widgets::label_row(ui, label, LABEL, |ui| {
        chosen = widgets::dropdown(ui, id, labels.get(current).copied().unwrap_or(""), labels, width);
    });
    chosen
}

/// A number row: `label`, then a field holding `key` (`unit`) kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, unit: &str, decimals: usize, range: (f64, f64)) {
    widgets::label_row(ui, label, LABEL, |ui| {
        let value = d.f64(key, range.0);
        if let Some(v) = widgets::plain_field(ui, ("photo-stitch", key), value, unit, decimals, NUMBER_W) {
            let v = if decimals == 0 { v.round() } else { v };
            d.fields.insert(key.into(), json!(v.clamp(range.0, range.1)));
        }
    });
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("Photo File:"), LABEL, |ui| {
        super::form::text(ui, d, "file", 260.0);
    });
    let provider = d.fields.get("provider").and_then(Value::as_str);
    let labels: Vec<&str> = PROVIDERS.iter().map(|p| p.1).collect();
    let current = PROVIDERS.iter().position(|p| p.0 == provider).unwrap_or(0);
    if let Some(p) = pick(ui, "photo-stitch-provider", tl!("Prepare With:"), current, &labels, 260.0).and_then(|i| PROVIDERS.get(i)) {
        d.fields.insert("provider".into(), json!(p.0));
    }
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Design"));
    let labels: Vec<&str> = FILLS.iter().map(|f| f.1).collect();
    let current = FILLS.iter().position(|f| f.0 == d.str("fill")).unwrap_or(0);
    if let Some(f) = pick(ui, "photo-stitch-fill", tl!("Fill:"), current, &labels, 260.0).and_then(|i| FILLS.get(i)) {
        let was = d.str("fill");
        d.fields.insert("fill".into(), json!(f.0));
        // Rows and the meander sew a line, not a fill: their spacing starts wider.
        let mono = |f: &str| matches!(f, "rows" | "meander");
        if mono(f.0) != mono(&was) {
            d.fields.insert("spacing".into(), json!(if mono(f.0) { 1.0 } else { 0.4 }));
        }
    }
    let colour = !matches!(d.str("fill").as_str(), "rows" | "meander");
    if colour {
        number(ui, d, "colors", tl!("Thread Colours:"), "", 0, (7.0, 16.0));
    }
    number(ui, d, "widthMm", tl!("Width:"), " mm", 1, (10.0, 1000.0));
    number(ui, d, "spacing", tl!("Spacing:"), " mm", 2, (0.2, 5.0));
    if colour {
        number(ui, d, "feather", tl!("Feather:"), " mm", 1, (0.0, 10.0));
        super::document_setup::check(ui, d, "features", tl!("Eye and Nose Detail"));
    }
    number(ui, d, "seed", tl!("Seed:"), "", 0, (0.0, 1_000_000.0));
    ui.add_space(8.0);
    super::form::caption(ui, tl!("The design opens as a new document. Gemini sends the photo to Google; the other choices work on this computer."));
    let error = d.str("__error");
    if !error.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(error).color(Tokens::get(ui.ctx()).error));
    }
    false
}

/// Run `photo.toStitches`; an error stays in the dialog.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = command_params(d);
    if params.get("file").and_then(Value::as_str).is_none_or(str::is_empty) {
        let e = "choose a photo file".to_string();
        if let Some(open) = app.ui.dialog.as_mut() {
            open.fields.insert("__error".into(), json!(e));
        }
        return Err(e);
    }
    match app.run("photo.toStitches", params) {
        Ok(v) => {
            app.ui.dialog = None;
            Ok(v)
        }
        Err(e) => {
            if let Some(open) = app.ui.dialog.as_mut() {
                open.fields.insert("__error".into(), json!(e));
            }
            Err(e)
        }
    }
}
