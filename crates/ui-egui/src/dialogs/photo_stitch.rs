//! Embroidery → Photo to Stitches…: turn a photo into a stitch design. The dialog only holds the
//! params of `photo.toStitches` (the photo's file, provider, thread colours, width, fill, spacing,
//! feather, the eye and nose detail pass and the seed); OK runs the command, which opens the design
//! as a new document, and closes. A command error stays in the dialog as its message.
//!
//! The photo is typed, chosen with Choose… (the host's open dialog, started in the folder of the
//! last photo) or dropped on the window while the dialog is open (the drop fills the field and opens
//! nothing). A photo that is there shows a thumbnail and its size in pixels under the field.
//!
//! Fields: `file`, `provider` (null: the `photoProvider` preference), `colors`, `widthMm`, `fill`,
//! `spacing`, `feather`, `features`, `seed`, `__preview` (what the dialog knows of the file: its
//! `path`, and `width` and `height` or a `note` saying why not) and `__error` (the last error).
//! Spec: `docs/superpowers/specs/2026-11-12-embroidery-photo-second-pass.md`.

use serde_json::{Map, Value, json};

use super::DialogSpec;
use crate::FilePick;
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

/// The photo file types the dialog offers and takes: what `photo.toStitches` reads.
pub(crate) const PHOTO_EXTS: &[&str] = &["png", "jpg", "jpeg", "heic", "heif", "tif", "tiff", "webp"];
/// A photo larger than this is not read (as the command: `MAX_PHOTO_BYTES`).
const MAX_PHOTO_BYTES: usize = 64 << 20;
/// A photo of more pixels than this is sized from its header but not decoded for the thumbnail.
const MAX_PREVIEW_PIXELS: u64 = 64 << 20;
/// The thumbnail's longest side, in points.
const THUMB: f32 = 96.0;

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

/// The open dialog the Choose… button shows: the photo types, in the folder of the photo named now,
/// else of the last one.
fn photo_pick(app: &VectorcraftApp, d: &Dialog) -> FilePick {
    let named = std::path::Path::new(d.str("file").trim()).parent().map(|p| p.to_string_lossy().to_string()).filter(|p| !p.is_empty());
    FilePick { folder: named.or_else(|| app.ui.photo_folder.clone()), filters: vec![FilePick::filter("Photos", PHOTO_EXTS)], ..Default::default() }
}

/// Choose…: the picked photo goes into the field. Shown off the UI thread, its answer runs this
/// again on the open dialog ([`crate::picks::in_dialog`]).
pub(super) fn choose(app: &mut VectorcraftApp, d: &mut Dialog) {
    let r = crate::picks::in_dialog(app, d, |app, d| {
        let pick = photo_pick(app, d);
        if let Some(path) = crate::picks::open(app, &pick) {
            set_file(app, d, &path);
        }
        Ok(())
    });
    // Choosing a file doesn't fail.
    let _ = r;
}

/// `path` is the photo: it goes into `d`'s field, the error about the last one goes, and its folder
/// is where the next picker starts.
fn set_file(app: &mut VectorcraftApp, d: &mut Dialog, path: &str) {
    d.fields.insert("file".into(), json!(path));
    d.fields.remove("__error");
    if let Some(folder) = std::path::Path::new(path).parent().map(|p| p.to_string_lossy().to_string()).filter(|p| !p.is_empty()) {
        app.ui.photo_folder = Some(folder);
    }
}

/// Whether the open dialog takes dropped files ([`drop_files`]).
pub(crate) fn takes_drops(app: &VectorcraftApp) -> bool {
    app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND)
}

/// Files dropped on the window while Photo to Stitches is open: the first photo among them fills
/// the field. Nothing else opens: a file of another kind is said not to be a photo.
pub(crate) fn drop_files(app: &mut VectorcraftApp, paths: &[String]) {
    let is_photo = |p: &&String| {
        std::path::Path::new(p.as_str()).extension().and_then(|e| e.to_str()).is_some_and(|e| PHOTO_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
    };
    let Some(path) = paths.iter().find(is_photo) else {
        app.status(tl!("Drop a PNG, JPEG, HEIC, TIFF or WebP photo"));
        return;
    };
    let Some(mut d) = app.ui.dialog.take_if(|d| d.kind == KIND) else { return };
    set_file(app, &mut d, path);
    app.ui.dialog = Some(d);
}

/// The file's first bytes through the app's file services (an error without them, as on the web).
fn read_photo(app: &VectorcraftApp, path: &str) -> Result<Vec<u8>, String> {
    let s = &app.services;
    match (&s.read_head, &s.read) {
        (Some(head), _) => head(path, MAX_PHOTO_BYTES + 1),
        (None, Some(read)) => read(path),
        (None, None) => Err("this build can't read files".into()),
    }
}

/// The photo in `bytes` decoded, within an allocation of [`MAX_PREVIEW_PIXELS`].
fn decode_photo(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PREVIEW_PIXELS * 8);
    reader.limits(limits);
    Ok(reader.decode().map_err(|e| e.to_string())?.into_rgba8())
}

/// The size of the photo in `bytes` and a thumbnail of at most `side` pixels. A photo of too many
/// pixels is sized from its header only. A format the `image` crate lacks (HEIC) is read by the
/// engine's reader, which gives PNG.
pub(super) fn thumbnail(bytes: &[u8], side: u32) -> Result<((u32, u32), Option<egui::ColorImage>), String> {
    let header = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?.into_dimensions();
    let (size, img) = match header {
        Ok((w, h)) if u64::from(w) * u64::from(h) > MAX_PREVIEW_PIXELS => return Ok(((w, h), None)),
        Ok(size) => (size, decode_photo(bytes)?),
        Err(_) => {
            let png = vectorcraft_engine::cmd::fileio::raster_image(bytes).map_err(|e| e.to_string())?;
            let img = decode_photo(&png.blob.bytes)?;
            ((img.width(), img.height()), img)
        }
    };
    let (w, h) = size;
    let scale = (f64::from(side) / f64::from(w.max(h).max(1))).min(1.0);
    let (tw, th) = (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1));
    let small = image::imageops::thumbnail(&img, tw, th);
    Ok((size, Some(egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], small.as_raw()))))
}

/// The id the thumbnail's texture is kept under, with the path it is of.
fn thumb_id() -> egui::Id {
    egui::Id::unique("photo-stitch-thumbnail")
}

type Thumb = (String, egui::TextureHandle);

/// Keep `__preview` (and the thumbnail's texture) in step with the file field: the file is read
/// again only when its path changed. A path that isn't a readable photo says why.
fn refresh_preview(app: &VectorcraftApp, ctx: &egui::Context, d: &mut Dialog) {
    let path = d.str("file").trim().to_string();
    if path.is_empty() {
        d.fields.remove("__preview");
        ctx.data_mut(|m| m.remove::<Thumb>(thumb_id()));
        return;
    }
    let old = d.fields.get("__preview");
    let known = old.and_then(|p| p.get("path")).and_then(Value::as_str) == Some(path.as_str());
    let has_texture = ctx.data(|m| m.get_temp::<Thumb>(thumb_id()).is_some_and(|(p, _)| p == path));
    // Done for this path: its thumbnail is kept, or there is none to keep (a note says why).
    if known && (has_texture || old.is_some_and(|p| p.get("note").is_some())) {
        return;
    }
    let mut preview = json!({"path": path});
    match read_photo(app, &path) {
        Err(_) => preview["note"] = json!(tl!("not a file that can be read")),
        Ok(bytes) if bytes.len() > MAX_PHOTO_BYTES => preview["note"] = json!(tl!("larger than 64 MB")),
        Ok(bytes) => match thumbnail(&bytes, 2 * THUMB as u32) {
            Err(e) => preview["note"] = json!(format!("{}: {e}", tl!("not a photo that can be read"))),
            Ok(((w, h), image)) => {
                preview["width"] = json!(w);
                preview["height"] = json!(h);
                match image {
                    Some(image) => {
                        let texture = ctx.load_texture("photo-stitch-thumbnail", image, egui::TextureOptions::LINEAR);
                        ctx.data_mut(|m| m.insert_temp(thumb_id(), (path.clone(), texture)));
                    }
                    None => preview["note"] = json!(tl!("too large for a thumbnail")),
                }
            }
        },
    }
    if preview.get("width").is_none() {
        ctx.data_mut(|m| m.remove::<Thumb>(thumb_id()));
    }
    d.fields.insert("__preview".into(), preview);
}

/// The thumbnail and the size under the file field, or the reason there is none.
fn show_preview(ui: &mut egui::Ui, d: &Dialog) {
    let Some(preview) = d.fields.get("__preview") else { return };
    let t = Tokens::get(ui.ctx());
    let size = preview.get("width").and_then(Value::as_u64).zip(preview.get("height").and_then(Value::as_u64));
    let note = preview.get("note").and_then(Value::as_str);
    let path = preview.get("path").and_then(Value::as_str).unwrap_or_default();
    let texture = ui.ctx().data(|m| m.get_temp::<Thumb>(thumb_id())).filter(|(p, _)| p == path).map(|(_, t)| t);
    ui.horizontal(|ui| {
        ui.add_space(LABEL);
        if let Some(texture) = &texture {
            let s = texture.size_vec2();
            let k = (THUMB / s.x.max(s.y)).min(1.0);
            ui.add(egui::Image::new((texture.id(), s * k)).corner_radius(3.0));
        }
        ui.vertical(|ui| {
            if let Some((w, h)) = size {
                ui.label(egui::RichText::new(format!("{w} × {h} px")).color(t.text));
            }
            if let Some(note) = note {
                ui.label(egui::RichText::new(note).size(11.5).color(t.text_dim));
            }
        });
    });
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

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("Photo File:"), LABEL, |ui| {
        super::form::text(ui, d, "file", 184.0);
        if crate::picks::can(app, &crate::picks::PickRequest::Open(FilePick::default())) && ui.button(tl!("Choose…")).clicked() {
            choose(app, d);
        }
    });
    refresh_preview(app, ui.ctx(), d);
    show_preview(ui, d);
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
