//! Embroidery › Production Worksheet… and Export for Production…: thin dialogs on
//! `stitch.worksheet` and `stitch.exportProduction`, over the stitch options the last stitch export
//! used.
//!
//! Worksheet fields: `path` (the PDF), `machine`, `spm`. Production fields: `dir` (the folder),
//! `name` (the files' base name; empty: the design name), `machine`, `dst`, `dsb`, `exp` (the
//! formats written), `worksheet`. Both keep `__params`, the stitch options they were opened with.
//! OK runs the command; an error keeps the dialog open.

use serde_json::{Map, Value, json};

use super::{DialogSpec, form, stitch_options};
use crate::state::Dialog;
use crate::{FilePick, VectorcraftApp, widgets};

pub(super) const WORKSHEET: &str = "stitchWorksheet";
pub(super) const PRODUCTION: &str = "stitchProduction";

pub(super) const WORKSHEET_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Production Worksheet").into(),
    body: worksheet_body,
    confirm: worksheet_confirm,
    ok: Some("Save"),
    min_width: 420.0,
    max_width: Some(480.0),
    ..DialogSpec::FORM
};

pub(super) const PRODUCTION_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Export for Production").into(),
    body: production_body,
    confirm: production_confirm,
    ok: Some("Export"),
    min_width: 440.0,
    max_width: Some(480.0),
    ..DialogSpec::FORM
};

const LABEL: f32 = 110.0;

/// The machines: (id, label), as Stitch File Options names them.
const MACHINES: [(&str, &str); 3] =
    [("bai_mirror", "Single-head, 15 needles (reads DST, DSB)"), ("commercial15", "15 needles, 500 × 350 mm field"), ("generic", "Generic")];

/// The stitch options last used, with `p` over them.
fn options(app: &VectorcraftApp, p: &Value) -> Value {
    let fields = stitch_options::fields(app, "dst", p);
    let mut params = stitch_options::export_params(&Dialog::new(stitch_options::KIND, Value::Object(fields)));
    if let Some(o) = params.as_object_mut() {
        // The format is the command's choice here.
        o.remove("format");
    }
    params
}

/// The design name: the document's title without its extension.
fn design_name(app: &VectorcraftApp) -> String {
    let title = app.session.active().map(|st| st.doc.title.clone()).unwrap_or_default();
    std::path::Path::new(&title).file_stem().map_or_else(|| "design".into(), |s| s.to_string_lossy().into_owned())
}

/// Open Production Worksheet…: `p` is `{path?, machine?, spm?, …stitch options}`.
pub fn open_worksheet(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let params = options(app, p);
    let machine = params.get("machine").and_then(Value::as_str).unwrap_or("bai_mirror").to_string();
    let path = p.get("path").and_then(Value::as_str).map_or_else(|| format!("{}-worksheet.pdf", design_name(app)), str::to_string);
    let spm = p.get("spm").cloned().unwrap_or(Value::Null);
    app.ui.dialog = Some(Dialog::new(WORKSHEET, json!({"path": path, "machine": machine, "spm": spm, "__params": params})));
    Ok(Value::Null)
}

/// Open Export for Production…: `p` is `{dir?, name?, formats?, worksheet?, machine?, …stitch options}`.
pub fn open_production(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let params = options(app, p);
    let machine = params.get("machine").and_then(Value::as_str).unwrap_or("bai_mirror").to_string();
    let doc_dir =
        app.session.active().and_then(|st| st.path.clone()).and_then(|p| std::path::Path::new(&p).parent().map(|d| d.to_string_lossy().into_owned()));
    let dir = p.get("dir").and_then(Value::as_str).map(str::to_string).or(doc_dir).unwrap_or_default();
    let formats: Vec<String> = match p.get("formats").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None if machine == "bai_mirror" => vec!["dst".into(), "dsb".into()],
        None => vec!["dst".into()],
    };
    let mut fields = Map::new();
    for f in ["dst", "dsb", "exp"] {
        fields.insert(f.into(), json!(formats.iter().any(|x| x == f)));
    }
    fields.insert("dir".into(), json!(dir));
    fields.insert("name".into(), p.get("name").cloned().unwrap_or(json!("")));
    fields.insert("machine".into(), json!(machine));
    fields.insert("worksheet".into(), p.get("worksheet").cloned().unwrap_or(json!(true)));
    fields.insert("__params".into(), params);
    app.ui.dialog = Some(Dialog::new(PRODUCTION, Value::Object(fields)));
    Ok(Value::Null)
}

fn machine_row(ui: &mut egui::Ui, d: &mut Dialog) {
    let labels: Vec<&str> = MACHINES.iter().map(|m| m.1).collect();
    let current = MACHINES.iter().position(|m| m.0 == d.str("machine")).unwrap_or(0);
    let mut picked = None;
    widgets::label_row(ui, tl!("Machine:"), LABEL, |ui| {
        picked = widgets::dropdown(ui, "stitch-production-machine", labels.get(current).copied().unwrap_or(""), &labels, 280.0);
    });
    if let Some(m) = picked.and_then(|i| MACHINES.get(i)) {
        d.fields.insert("machine".into(), json!(m.0));
    }
}

/// The command's params: the stitch options with the dialog's fields over them.
fn params(d: &Dialog, keys: &[&str]) -> Value {
    let mut p = d.fields.get("__params").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    for k in keys {
        if let Some(v) = d.fields.get(*k).filter(|v| !v.is_null() && v.as_str() != Some("")) {
            p[*k] = v.clone();
        }
    }
    p
}

fn worksheet_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("File:"), LABEL, |ui| {
        form::text(ui, d, "path", 230.0);
        if ui.button(tl!("Choose…")).clicked() {
            let name = d.str("path");
            let r = crate::picks::in_dialog(app, d, move |app, d| {
                let pick = FilePick::named(
                    std::path::Path::new(&name).file_name().map_or("worksheet.pdf".into(), |n| n.to_string_lossy().into_owned()).as_str(),
                );
                if let Some(path) = crate::picks::save(app, &pick) {
                    d.fields.insert("path".into(), json!(path));
                }
                Ok(())
            });
            // Picking a file doesn't fail.
            let _ = r;
        }
    });
    machine_row(ui, d);
    widgets::label_row(ui, tl!("Speed:"), LABEL, |ui| {
        let value = d.fields.get("spm").and_then(Value::as_f64).unwrap_or(if d.str("machine") == "bai_mirror" { 1200.0 } else { 800.0 });
        if let Some(v) = widgets::plain_field(ui, "stitch-worksheet-spm", value, " spm", 0, 80.0) {
            d.fields.insert("spm".into(), json!(v.round().clamp(100.0, 3000.0)));
        }
    });
    ui.add_space(6.0);
    form::caption(
        ui,
        tl!(
            "One A4 page for the operator: the design picture, size, stitch count, hoop, run time, thread and bobbin estimates, and the stop table with thread codes and needles."
        ),
    );
    false
}

fn worksheet_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.str("path").trim().is_empty() {
        return Err(tl!("Choose a file for the worksheet.").into());
    }
    let r = app.run("stitch.worksheet", params(d, &["path", "machine", "spm"]))?;
    app.ui.dialog = None;
    app.status(crate::i18n::fmt(tl!("Saved the worksheet to {path}"), &[("path", d.str("path").as_str())]));
    Ok(r)
}

fn production_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("Folder:"), LABEL, |ui| {
        form::text(ui, d, "dir", 230.0);
        if app.services.pick_folder.is_some() && ui.button(tl!("Choose…")).clicked() {
            crate::picks::folder_field(app, d, "dir");
        }
    });
    widgets::label_row(ui, tl!("File Name:"), LABEL, |ui| {
        form::text(ui, d, "name", 230.0);
    });
    machine_row(ui, d);
    ui.add_space(6.0);
    widgets::subheader(ui, tl!("Files"));
    for (k, label) in [
        ("dst", tl!("DST machine file")),
        ("dsb", tl!("DSB machine file")),
        ("exp", tl!("EXP machine file")),
        ("worksheet", tl!("Production worksheet (PDF)")),
    ] {
        let on = d.bool(k);
        if widgets::check(ui, label, on, true) {
            d.fields.insert(k.into(), json!(!on));
        }
    }
    ui.add_space(6.0);
    form::caption(ui, tl!("Files of the same names in the folder are replaced. The design check runs too; its findings show in the status bar."));
    false
}

fn production_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.str("dir").trim().is_empty() {
        return Err(tl!("Choose a folder.").into());
    }
    let mut p = params(d, &["dir", "name", "machine", "worksheet"]);
    let formats: Vec<&str> = ["dst", "dsb", "exp"].into_iter().filter(|f| d.bool(f)).collect();
    p["formats"] = json!(formats);
    let r = app.run("stitch.exportProduction", p)?;
    app.ui.dialog = None;
    let files = r.get("files").and_then(Value::as_array).map_or(0, Vec::len);
    let problems = r
        .get("findings")
        .and_then(Value::as_array)
        .map_or(0, |f| f.iter().filter(|x| x.get("level").and_then(Value::as_str) != Some("info")).count());
    let (files, problems) = (files.to_string(), problems.to_string());
    app.status(crate::i18n::fmt(
        tl!("Wrote {files} files to {dir}; the design check found {problems} warnings or errors"),
        &[("files", files.as_str()), ("dir", d.str("dir").as_str()), ("problems", problems.as_str())],
    ));
    Ok(r)
}
