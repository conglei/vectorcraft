//! Photo preparation for embroidery (gap E12): `photo.providers` lists who can prepare a photo,
//! `photo.prepare` prepares one and checks it still looks like the photo.
//!
//! The engine never talks to the network. The built-in provider, `nomodel`, works on this computer
//! alone ([`vectorcraft_imageprep::NoModel`]); a host app installs others, such as the Gemini
//! adapter of `vectorcraft-imagemodel`, with [`Providers::install`]. A provider that sends the photo
//! off the computer runs only when the params name it or the `photoProvider` preference chooses it.
//! Spec: `docs/superpowers/specs/2026-11-11-embroidery-image-provider-design.md`.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_imageprep::{
    DEFAULT_MAX_SIDE, ImageProvider, LIKENESS_THRESHOLD, MAX_MAX_SIDE, MIN_MAX_SIDE, NO_MODEL, NoModel, PrepareOptions, Raster, encode_png, likeness,
};

use super::{CommandSpec, Result, Session, always, bad, cmd, number_or, switch_or, text_param};

const PREPARE: &str = "photo.prepare";
/// The largest photo file read (64 MB).
pub const MAX_PHOTO_BYTES: usize = 64 << 20;
/// The largest prepared image returned inline as base64 (2048 × 2048 pixels); larger ones need `out`.
const MAX_INLINE_PIXELS: u64 = 2048 * 2048;

/// The `photoProvider` preference's choices: (value, label).
pub const PROVIDER_CHOICES: &[(&str, &str)] = &[("nomodel", "No Model (on this computer)"), ("gemini", "Gemini (sends the photo to Google)")];

/// The providers a host installed besides the built-in `nomodel` ([`crate::Session::photo`]).
#[derive(Default)]
pub struct Providers {
    installed: Vec<Arc<dyn ImageProvider>>,
}

impl Providers {
    /// Install `p`, replacing an installed provider of the same name. `nomodel` is built in and
    /// can't be replaced.
    pub fn install(&mut self, p: Arc<dyn ImageProvider>) {
        if p.name() == NO_MODEL {
            return;
        }
        self.installed.retain(|q| q.name() != p.name());
        self.installed.push(p);
    }

    /// The provider called `name`, if there is one.
    pub fn find(&self, name: &str) -> Option<Arc<dyn ImageProvider>> {
        if name == NO_MODEL {
            return Some(Arc::new(NoModel));
        }
        self.installed.iter().find(|p| p.name() == name).cloned()
    }

    /// Every provider: `nomodel` first, then the installed ones in the order installed.
    pub fn all(&self) -> Vec<Arc<dyn ImageProvider>> {
        let mut v: Vec<Arc<dyn ImageProvider>> = vec![Arc::new(NoModel)];
        v.extend(self.installed.iter().cloned());
        v
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "photo.providers",
            "Photo Providers",
            [],
            None,
            "{} the providers photo.prepare can use → {providers: [{name, ready (it can run now), sendsOffMachine (it sends the photo off this computer), note? (why it can't run)}], active (what photo.prepare uses when its params name no provider: the photoProvider preference, or nomodel when that provider can't run), preference (photoProvider)}; nomodel is always there; gemini only where the app installed it (not on the web)",
            always,
            providers
        ),
        cmd!(
            query "photo.prepare",
            "Prepare Photo",
            [],
            None,
            "{file: (the photo: a PNG, JPEG, WebP, GIF, TIFF or BMP file, at most 64 MB and 192 megapixels), path?: (write the prepared PNG there), provider?: nomodel | gemini (not given: the photoProvider preference, default nomodel; naming gemini, or choosing it in the preference, is the opt-in to sending the photo to Google's Gemini API, which needs GEMINI_API_KEY: named here without a key it is an error, chosen in the preference without one it falls back to nomodel with a warning), maxSide?: 1024 (pixels, 64–4096: the longest side of the result; the photo is never scaled up; Gemini is sent at most 1536), keepAlpha?: true (false: flattened onto white), cleanUp?: true (nomodel: smooth the noise within tones, edges kept), likenessThreshold?: 0.6 (0–1)} prepares a photo faithfully for embroidery (the subject kept as it is; Gemini also removes the background, nomodel can't), then compares it with the photo → {provider, width, height, likeness (0–1: structural similarity of the subject plus a colour term; below likenessThreshold a warning), notes, warnings, path}; without path {…, pngBase64} (at most 2048 × 2048 pixels: pass path or a lower maxSide for more); a file that isn't an image, a provider that isn't installed or fails, is an error",
            always,
            prepare
        ),
    ]
}

/// The provider `photo.prepare` uses without a `provider` param, and a warning when the
/// preference's provider can't run.
fn default_provider(s: &Session) -> (Arc<dyn ImageProvider>, Option<String>) {
    let pref = s.prefs.photo_provider.as_str();
    if pref == NO_MODEL {
        return (Arc::new(NoModel), None);
    }
    match s.photo.find(pref) {
        Some(p) => match p.ready() {
            Ok(()) => (p, None),
            Err(why) => (Arc::new(NoModel), Some(format!("the photoProvider preference chooses {pref}, but {why}: prepared without a model"))),
        },
        None => {
            (Arc::new(NoModel), Some(format!("the photoProvider preference chooses {pref}, which this app doesn't have: prepared without a model")))
        }
    }
}

fn providers(s: &mut Session, _: &Value) -> Result<Value> {
    let list: Vec<Value> = s
        .photo
        .all()
        .iter()
        .map(|p| {
            let mut v = json!({"name": p.name(), "ready": p.ready().is_ok(), "sendsOffMachine": p.sends_off_machine()});
            if let Err(note) = p.ready() {
                v["note"] = json!(note);
            }
            v
        })
        .collect();
    let (active, _) = default_provider(s);
    Ok(json!({"providers": list, "active": active.name(), "preference": s.prefs.photo_provider}))
}

/// The prepare options in `p`.
fn options(p: &Value) -> Result<PrepareOptions> {
    let side = number_or(p, "maxSide", f64::from(DEFAULT_MAX_SIDE), PREPARE)?;
    let (lo, hi) = (f64::from(MIN_MAX_SIDE), f64::from(MAX_MAX_SIDE));
    if !(side.is_finite() && (lo..=hi).contains(&side)) {
        return Err(bad(PREPARE, format!("maxSide must be {MIN_MAX_SIDE}–{MAX_MAX_SIDE} pixels, not {side}")));
    }
    Ok(PrepareOptions {
        max_side: side.round() as u32,
        keep_alpha: switch_or(p, "keepAlpha", true, PREPARE)?,
        clean_up: switch_or(p, "cleanUp", true, PREPARE)?,
        ..Default::default()
    })
}

/// The photo at `path`, decoded (scaled down to 64 megapixels when larger, up to 192).
fn read_photo(path: &str) -> Result<Raster> {
    let bytes = super::fileio::read_file_head(path, MAX_PHOTO_BYTES + 1)?;
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err(bad(PREPARE, format!("{path} is larger than {} MB", MAX_PHOTO_BYTES >> 20)));
    }
    Raster::decode_to_fit(&bytes).map_err(|e| bad(PREPARE, format!("{path}: {e}")))
}

fn prepare(s: &mut Session, p: &Value) -> Result<Value> {
    let file = text_param(p, "file", PREPARE)?.ok_or_else(|| bad(PREPARE, "file: the photo to prepare"))?;
    // `path`, as in stitch.preview and the exports: where the result is written.
    let out_path = text_param(p, "path", PREPARE)?;
    let o = options(p)?;
    let threshold = number_or(p, "likenessThreshold", LIKENESS_THRESHOLD, PREPARE)?;
    if !(0.0..=1.0).contains(&threshold) {
        return Err(bad(PREPARE, format!("likenessThreshold must be 0–1, not {threshold}")));
    }
    let mut warnings = Vec::new();
    let provider = match text_param(p, "provider", PREPARE)? {
        Some(name) => {
            let names: Vec<String> = s.photo.all().iter().map(|p| p.name().to_string()).collect();
            let found = s.photo.find(name).ok_or_else(|| bad(PREPARE, format!("provider `{name}`: {}", names.join(" or "))))?;
            found.ready().map_err(|why| bad(PREPARE, why))?;
            found
        }
        None => {
            let (found, warning) = default_provider(s);
            warnings.extend(warning);
            found
        }
    };
    if provider.name() == NO_MODEL && s.photo.all().iter().any(|q| q.sends_off_machine() && q.ready().is_ok()) {
        warnings.push("prepared without a model, so the background is still there: pass provider: gemini, or set photoProvider to gemini, to have it removed (the photo is then sent to Google's Gemini API)".to_string());
    }
    let photo = read_photo(file)?;
    let prepared = provider.prepare(&photo, &o).map_err(|e| bad(PREPARE, e))?;
    let (w, h) = (prepared.image.width, prepared.image.height);
    if out_path.is_none() && u64::from(w) * u64::from(h) > MAX_INLINE_PIXELS {
        return Err(bad(PREPARE, format!("a {w} × {h} result is too large to return inline: pass path, or a lower maxSide")));
    }
    let like = likeness(&photo, &prepared.image);
    if like.score < threshold {
        warnings.push(format!(
            "the prepared image looks less like the photo than expected (likeness {:.2}, below {threshold}): check it before stitching",
            like.score
        ));
    }
    let mut notes = prepared.notes;
    notes.extend(like.notes);
    let png = encode_png(&prepared.image).map_err(|e| bad(PREPARE, e))?;
    let mut out = json!({
        "provider": prepared.provider,
        "width": w,
        "height": h,
        "likeness": (like.score * 1000.0).round() / 1000.0,
        "notes": notes,
        "warnings": warnings,
    });
    match out_path {
        Some(o) => {
            super::fileio::write_file(o, &png)?;
            out["path"] = json!(o);
        }
        None => out["pngBase64"] = json!(vectorcraft_format::base64_encode(&png)),
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
