//! Embroidery → Auto-Digitize Image…: trace the selected image for embroidery and turn every region
//! into stitch objects (`image.autoDigitize`). The dialog only holds the command's few params (the
//! thread colours, the smallest region, the widest satin and whether the image is kept as a
//! template); OK runs it and then shows what it made, and OK again closes. A command error stays in
//! the dialog as its message.
//!
//! Fields: `colors`, `minRegionMm2`, `maxSatin`, `keepImage`, `__error` (the last error) and
//! `__result` (the summary once it ran).
//! Spec: `docs/superpowers/specs/2026-11-13-embroidery-auto-digitize.md`.

use serde_json::{Map, Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "autoDigitizeImage";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Auto-Digitize Image").into(),
    body,
    confirm,
    ok: Some("OK"),
    min_width: 400.0,
    max_width: Some(460.0),
    inputs: |_| &["colors", "minRegionMm2", "maxSatin", "keepImage"],
    ..DialogSpec::FORM
};

const LABEL: f32 = 170.0;
const NUMBER_W: f32 = 70.0;

/// The fields a dialog starts from: the command's defaults.
pub(crate) fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [("colors", json!(6)), ("minRegionMm2", json!(2)), ("maxSatin", json!(8)), ("keepImage", json!(true))] {
        m.insert(k.into(), v);
    }
    m
}

/// Open Auto-Digitize Image with `params` (any of the fields) over the defaults; an image must be
/// selected.
pub fn open(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let image = st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))));
    if !image {
        return Err("select an image".into());
    }
    let mut fields = defaults();
    if let Some(o) = params.as_object() {
        fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
    Ok(Value::Null)
}

/// The `image.autoDigitize` params the fields stand for.
pub(crate) fn command_params(d: &Dialog) -> Value {
    let mut p = Map::new();
    for key in ["colors", "minRegionMm2", "maxSatin"] {
        if let Some(v) = d.fields.get(key).and_then(Value::as_f64) {
            p.insert(key.into(), json!(v));
        }
    }
    p.insert("keepImage".into(), json!(d.bool("keepImage")));
    Value::Object(p)
}

/// A number row: `label`, then a field holding `key` (`unit`) kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, unit: &str, decimals: usize, range: (f64, f64)) {
    widgets::label_row(ui, label, LABEL, |ui| {
        let value = d.f64(key, range.0);
        if let Some(v) = widgets::plain_field(ui, ("auto-digitize", key), value, unit, decimals, NUMBER_W) {
            let v = if decimals == 0 { v.round() } else { v };
            d.fields.insert(key.into(), json!(v.clamp(range.0, range.1)));
        }
    });
}

/// The summary of what `image.autoDigitize` answered.
fn summary(r: &Value) -> String {
    let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0);
    format!(
        "Regions: {} in {} colours. Fills: {}, satin columns: {}, runs: {}. Colour changes: {}, trims: {}.",
        n("regions"),
        n("colors"),
        n("fills"),
        n("columns"),
        n("runs"),
        n("colorChanges"),
        n("trims")
    )
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let result = d.str("__result");
    if !result.is_empty() {
        ui.label(result);
        return false;
    }
    number(ui, d, "colors", tl!("Thread Colours:"), "", 0, (2.0, 12.0));
    number(ui, d, "minRegionMm2", tl!("Smallest Region:"), " mm²", 1, (0.0, 100.0));
    number(ui, d, "maxSatin", tl!("Widest Satin:"), " mm", 1, (2.0, 15.0));
    super::document_setup::check(ui, d, "keepImage", tl!("Keep the Image as a Hidden Template"));
    ui.add_space(8.0);
    super::form::caption(ui, tl!("Thin parts become running stitch, parts up to the widest satin satin columns, wider parts tatami fill."));
    let error = d.str("__error");
    if !error.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(error).color(Tokens::get(ui.ctx()).error));
    }
    false
}

/// Run `image.autoDigitize` and show what it made; once shown, OK closes. An error stays in the
/// dialog.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if !d.str("__result").is_empty() {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    match app.run("image.autoDigitize", command_params(d)) {
        Ok(v) => {
            if let Some(open) = app.ui.dialog.as_mut() {
                open.fields.insert("__result".into(), json!(summary(&v)));
                open.fields.remove("__error");
            }
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
