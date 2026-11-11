//! Stitch File Options (Export As → DST or EXP): the machine, how paths become stitches (running
//! stitch along each stroked path, or one needle point per anchor), the stitch lengths, lock
//! stitches, how many jumps in a row make a trim, trims before colour changes, where the file
//! starts and ends, the design name, and whether only the selection is stitched.
//!
//! Fields: `document.export` params (`format: dst | exp`, `path?`) and the stitch options of
//! `stitch.info`. `mode` and `locks` are null while they are left to the engine (it decides per
//! layer: art on the layer an opened stitch file made keeps its needle points and its own locks).
//! OK remembers the options for next time and writes the file (asking where when there is no
//! path); bad options keep the dialog open.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "stitchOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Stitch File Options").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 440.0,
    max_width: Some(480.0),
    inputs: |_| &["path"],
    ..DialogSpec::FORM
};

const LABEL: f32 = 150.0;
/// The field width of the number fields.
const NUMBER_W: f32 = 70.0;

/// The longest stitch a DST file holds, in mm, and an EXP file (its own limit is also its default).
pub(crate) const MAX_STITCH_DST: f64 = 12.1;
pub(crate) const MAX_STITCH_EXP: f64 = 12.7;

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 6] = ["format", "path", "useArtboards", "range", "artboard", "selectedOnly"];

/// The machines: (id, label).
const MACHINES: [(&str, &str); 3] = [
    ("commercial15", "15 needles, 500 × 350 mm field"),
    ("bai_mirror", "Single-head, 15 needles (reads DST)"),
    ("generic", "Generic, no needle or field limits"),
];
/// How filled paths are sewn: (`fill.type`, label).
const FILLS: [(&str, &str); 2] = [("tatami", "Tatami fill"), ("none", "Not sewn")];
/// How stroked paths are sewn: (`stroke.type`, label).
const STROKES: [(&str, &str); 2] = [("run", "Running stitch"), ("satin", "Satin column")];
/// The ways paths become stitches: (`mode`, label); null is left to the engine.
const MODES: [(Option<&str>, &str); 3] =
    [(None, "Automatic"), (Some("run"), "Running stitch along each path"), (Some("manual"), "One needle point per anchor")];
/// Lock stitches: (`locks`, label); null is left to the engine.
const LOCKS: [(Option<bool>, &str); 3] = [(None, "Automatic"), (Some(true), "On"), (Some(false), "Off")];
/// Where the file's origin is: (id, label).
const STARTS: [(&str, &str); 2] = [("center", "Center of the design"), ("firstStitch", "First stitch")];

/// The longest stitch `format` holds, in mm.
pub(crate) fn max_stitch(format: &str) -> f64 {
    if format == "exp" { MAX_STITCH_EXP } else { MAX_STITCH_DST }
}

/// The options a first Stitch File Options dialog starts from (the export's defaults).
pub(crate) fn defaults(format: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [
        ("machine", json!("commercial15")),
        ("mode", Value::Null),
        ("stitchLength", json!(2.5)),
        ("minStitch", json!(0.3)),
        ("maxStitch", json!(max_stitch(format))),
        ("locks", Value::Null),
        ("jumpsPerTrim", json!(3)),
        ("trimBeforeColorChange", json!(false)),
        ("start", json!("center")),
        ("returnToStart", json!(true)),
        ("name", json!("")),
        ("selectedOnly", json!(false)),
        ("fillType", json!("tatami")),
        ("strokeType", json!("run")),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// The fields of a dialog for `format`: the defaults, the options used last, then `params`; the
/// longest stitch never exceeds what the format holds.
pub(crate) fn fields(app: &VectorcraftApp, format: &str, params: &Value) -> Map<String, Value> {
    let mut fields = defaults(format);
    for given in [&app.ui.stitch_options, params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    fields.insert("format".into(), json!(format));
    let max = max_stitch(format);
    if let Some(v) = fields.get("maxStitch").and_then(Value::as_f64)
        && v > max
    {
        fields.insert("maxStitch".into(), json!(max));
    }
    fields
}

/// The `document.export` / `stitch.info` params the fields stand for: what is left to the engine
/// (a null, and an empty name) is left out.
pub(crate) fn export_params(d: &Dialog) -> Value {
    let mut p = super::form::params(d);
    if let Some(o) = p.as_object_mut() {
        o.retain(|_, v| !v.is_null());
        if o.get("name").and_then(Value::as_str).is_some_and(|n| n.trim().is_empty()) {
            o.remove("name");
        }
        // The stitch types are the `fill` and `stroke` option objects, given when not the default.
        for (key, object, default) in [("fillType", "fill", "tatami"), ("strokeType", "stroke", "run")] {
            if let Some(t) = o.remove(key).as_ref().and_then(Value::as_str).filter(|t| *t != default) {
                o.insert(object.into(), json!({ "type": t }));
            }
        }
        if o.get("mode").and_then(Value::as_str) == Some("auto") {
            o.remove("mode");
        }
    }
    p
}

/// Open Stitch File Options for an export whose params (`format`, `path`, any stitch option) are
/// `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let format = match params.get("format").and_then(Value::as_str) {
        Some("exp") => "exp",
        _ => "dst",
    };
    let fields = fields(app, format, params);
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

/// A dropdown row: `label`, then the dropdown showing `current` among `labels`. Returns the index
/// picked.
fn pick(ui: &mut egui::Ui, id: &str, label: &str, current: usize, labels: &[&str], width: f32) -> Option<usize> {
    let mut chosen = None;
    widgets::label_row(ui, label, LABEL, |ui| {
        chosen = widgets::dropdown(ui, id, labels.get(current).copied().unwrap_or(""), labels, width);
    });
    chosen
}

/// A number row: `label`, then a field holding `key` in mm (`unit`) kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, unit: &str, decimals: usize, range: (f64, f64)) {
    widgets::label_row(ui, label, LABEL, |ui| {
        let value = d.f64(key, range.0);
        if let Some(v) = widgets::plain_field(ui, ("stitch-option", key), value, unit, decimals, NUMBER_W) {
            let v = if decimals == 0 { v.round() } else { v };
            d.fields.insert(key.into(), json!(v.clamp(range.0, range.1)));
        }
    });
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let format = if d.str("format") == "exp" { "exp" } else { "dst" };
    let labels: Vec<&str> = MACHINES.iter().map(|m| m.1).collect();
    let machine = MACHINES.iter().position(|m| m.0 == d.str("machine")).unwrap_or(0);
    if let Some(i) = pick(ui, "stitch-machine", tl!("Machine:"), machine, &labels, 260.0).and_then(|i| MACHINES.get(i)) {
        d.fields.insert("machine".into(), json!(i.0));
    }
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Stitches"));
    let mode = d.fields.get("mode").and_then(Value::as_str).filter(|m| *m != "auto");
    let labels: Vec<&str> = MODES.iter().map(|m| m.1).collect();
    let current = MODES.iter().position(|m| m.0 == mode).unwrap_or(0);
    if let Some(m) = pick(ui, "stitch-mode", tl!("Stitch Mode:"), current, &labels, 260.0).and_then(|i| MODES.get(i)) {
        d.fields.insert("mode".into(), json!(m.0));
    }
    number(ui, d, "stitchLength", tl!("Stitch Length:"), " mm", 2, (0.5, 12.0));
    number(ui, d, "minStitch", tl!("Minimum Stitch:"), " mm", 2, (0.0, 2.0));
    number(ui, d, "maxStitch", tl!("Maximum Stitch:"), " mm", 2, (1.0, max_stitch(format)));
    let locks = d.fields.get("locks").and_then(Value::as_bool);
    let labels: Vec<&str> = LOCKS.iter().map(|l| l.1).collect();
    let current = LOCKS.iter().position(|l| l.0 == locks).unwrap_or(0);
    if let Some(l) = pick(ui, "stitch-locks", tl!("Lock Stitches:"), current, &labels, 120.0).and_then(|i| LOCKS.get(i)) {
        d.fields.insert("locks".into(), json!(l.0));
    }
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Stitch types"));
    for (key, id, label, options) in
        [("fillType", "stitch-fill", tl!("Filled Paths:"), &FILLS), ("strokeType", "stitch-stroke", tl!("Stroked Paths:"), &STROKES)]
    {
        let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
        let current = options.iter().position(|o| o.0 == d.str(key)).unwrap_or(0);
        if let Some(o) = pick(ui, id, label, current, &labels, 200.0).and_then(|i| options.get(i)) {
            d.fields.insert(key.into(), json!(o.0));
        }
    }
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Trims and Colors"));
    number(ui, d, "jumpsPerTrim", tl!("Jumps per Trim:"), "", 0, (2.0, 8.0));
    super::document_setup::check(ui, d, "trimBeforeColorChange", tl!("Trim Before Each Color Change"));
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("File"));
    let labels: Vec<&str> = STARTS.iter().map(|s| s.1).collect();
    let current = STARTS.iter().position(|s| s.0 == d.str("start")).unwrap_or(0);
    if let Some(s) = pick(ui, "stitch-start", tl!("Origin:"), current, &labels, 200.0).and_then(|i| STARTS.get(i)) {
        d.fields.insert("start".into(), json!(s.0));
    }
    super::document_setup::check(ui, d, "returnToStart", tl!("Return to the Origin at the End"));
    widgets::label_row(ui, tl!("Design Name:"), LABEL, |ui| {
        super::form::text(ui, d, "name", 200.0);
    });
    ui.add_space(4.0);
    let selection = app.session.active().is_some_and(|st| !st.selection.is_empty());
    if widgets::check(ui, tl!("Stitch Selected Art Only"), d.bool("selectedOnly") && selection, selection) {
        d.fields.insert("selectedOnly".into(), json!(!d.bool("selectedOnly")));
    }
    ui.add_space(8.0);
    super::form::caption(
        ui,
        if format == "exp" {
            tl!("Each path is sewn in its fill or stroke color, one thread per color. EXP holds moves of up to 12.7 mm.")
        } else {
            tl!("Each path is sewn in its fill or stroke color, one thread per color. DST holds moves of up to 12.1 mm.")
        },
    );
    false
}

/// Check the options, remember them, and write the file.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = export_params(d);
    let format = if d.str("format") == "exp" { "exp" } else { "dst" };
    fileio::check_stitch_options(&params, format)?;
    // Nothing selected keeps the dialog open too.
    if let Some(st) = app.session.active() {
        fileio::export_source(st, &params).map_err(|e| e.to_string())?;
    }
    app.ui.stitch_options = Value::Object(
        d.fields.iter().filter(|(k, _)| !k.starts_with("__") && !PER_EXPORT.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect(),
    );
    app.ui.dialog = None;
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    io::export(app, Some(format), path, &params)
}
