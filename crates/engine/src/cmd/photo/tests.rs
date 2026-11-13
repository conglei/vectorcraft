use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use vectorcraft_imageprep::{ImageProvider, PrepareOptions, Prepared, Raster, encode_png, synthetic_photo};

use crate::Session;

/// A provider standing in for a hosted model: no network. `key` says whether it is ready;
/// `blank` makes it return a blank image (a failed preparation).
struct Fake {
    key: bool,
    blank: bool,
    calls: AtomicUsize,
}

impl Fake {
    fn new(key: bool, blank: bool) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { key, blank, calls: AtomicUsize::new(0) })
    }
}

impl ImageProvider for Fake {
    fn name(&self) -> &str {
        "gemini"
    }
    fn sends_off_machine(&self) -> bool {
        true
    }
    fn ready(&self) -> Result<(), String> {
        if self.key { Ok(()) } else { Err("set GEMINI_API_KEY to use the Gemini provider; the no-model path still works".into()) }
    }
    fn prepare(&self, image: &Raster, _: &PrepareOptions) -> Result<Prepared, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let img = if self.blank { Raster::from_fn(image.width, image.height, |_, _| [255, 255, 255, 255]) } else { image.clone() };
        Ok(Prepared { image: img, provider: "gemini".into(), notes: vec!["fake".into()] })
    }
}

/// A photo made in code, written as a PNG to a temp file of its own.
fn photo_file(name: &str, w: u32, h: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-photo-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("photo.png");
    std::fs::write(&path, encode_png(&synthetic_photo(w, h, 9)).unwrap()).unwrap();
    path
}

fn prepare(s: &mut Session, p: Value) -> Result<Value, String> {
    s.execute("photo.prepare", &p).map_err(|e| e.to_string())
}

fn texts(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect()
}

#[test]
fn providers_lists_nomodel_and_what_the_host_installed() {
    let mut s = Session::new();
    let r = s.execute("photo.providers", &json!({})).unwrap();
    assert_eq!(r["active"], "nomodel");
    assert_eq!(r["preference"], "nomodel");
    assert_eq!(r["providers"], json!([{"name": "nomodel", "ready": true, "sendsOffMachine": false}]));
    s.photo.install(Fake::new(false, false));
    let r = s.execute("photo.providers", &json!({})).unwrap();
    let g = &r["providers"][1];
    assert_eq!((g["name"].as_str(), g["ready"].as_bool(), g["sendsOffMachine"].as_bool()), (Some("gemini"), Some(false), Some(true)));
    assert!(g["note"].as_str().unwrap().contains("GEMINI_API_KEY"));
    // The preference chooses gemini, but it can't run: nomodel stays active.
    s.execute("prefs.set", &json!({"key": "photoProvider", "value": "gemini"})).unwrap();
    assert_eq!(s.execute("photo.providers", &json!({})).unwrap()["active"], "nomodel");
    s.photo.install(Fake::new(true, false));
    assert_eq!(s.execute("photo.providers", &json!({})).unwrap()["active"], "gemini");
    assert!(s.execute("prefs.set", &json!({"key": "photoProvider", "value": "openai"})).is_err());
}

#[test]
fn nomodel_prepares_a_photo_end_to_end() {
    let path = photo_file("nomodel", 1600, 1200);
    let mut s = Session::new();
    let r = prepare(&mut s, json!({"file": path.to_string_lossy()})).unwrap();
    assert_eq!(r["provider"], "nomodel");
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1024), Some(768)));
    assert!(r["likeness"].as_f64().unwrap() >= 0.6, "{r}");
    assert!(texts(&r["warnings"]).is_empty(), "{r}");
    assert!(texts(&r["notes"]).iter().any(|n| n.contains("background was not removed")), "{r}");
    let png = vectorcraft_format::base64_decode(r["pngBase64"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (1024, 768));
    // To a file, at another size.
    let out = path.with_file_name("prepared.png");
    let r = prepare(&mut s, json!({"file": path.to_string_lossy(), "path": out.to_string_lossy(), "maxSide": 400, "cleanUp": false})).unwrap();
    assert!(r.get("pngBase64").is_none());
    assert_eq!(r["path"], json!(out.to_string_lossy()));
    let img = image::open(&out).unwrap();
    assert_eq!((img.width(), img.height()), (400, 300));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_hosted_provider_runs_only_when_named_or_chosen() {
    let path = photo_file("optin", 200, 150);
    let p = json!({"file": path.to_string_lossy()});
    let mut s = Session::new();
    let fake = Fake::new(true, false);
    s.photo.install(fake.clone());
    // Installed and ready, but nothing chose it: nomodel, with a hint.
    let r = prepare(&mut s, p.clone()).unwrap();
    assert_eq!(r["provider"], "nomodel");
    assert!(texts(&r["warnings"]).iter().any(|w| w.contains("provider: gemini")), "{r}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    // Named in the params.
    let r = prepare(&mut s, json!({"file": path.to_string_lossy(), "provider": "gemini"})).unwrap();
    assert_eq!(r["provider"], "gemini");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    // Chosen in the preference.
    s.prefs.photo_provider = "gemini".into();
    assert_eq!(prepare(&mut s, p.clone()).unwrap()["provider"], "gemini");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    // Named explicitly as nomodel: no call, even with the preference on gemini.
    assert_eq!(prepare(&mut s, json!({"file": path.to_string_lossy(), "provider": "nomodel"})).unwrap()["provider"], "nomodel");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn without_a_key_gemini_named_is_an_error_and_chosen_falls_back() {
    let path = photo_file("nokey", 200, 150);
    let mut s = Session::new();
    // Not installed (the web, tests): an error naming what there is.
    let e = prepare(&mut s, json!({"file": path.to_string_lossy(), "provider": "gemini"})).unwrap_err();
    assert!(e.contains("nomodel"), "{e}");
    let fake = Fake::new(false, false);
    s.photo.install(fake.clone());
    let e = prepare(&mut s, json!({"file": path.to_string_lossy(), "provider": "gemini"})).unwrap_err();
    assert!(e.contains("set GEMINI_API_KEY to use the Gemini provider; the no-model path still works"), "{e}");
    s.prefs.photo_provider = "gemini".into();
    let r = prepare(&mut s, json!({"file": path.to_string_lossy()})).unwrap();
    assert_eq!(r["provider"], "nomodel");
    assert!(texts(&r["warnings"]).iter().any(|w| w.contains("GEMINI_API_KEY") && w.contains("without a model")), "{r}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_result_unlike_the_photo_carries_a_warning() {
    let path = photo_file("unlike", 200, 150);
    let mut s = Session::new();
    s.photo.install(Fake::new(true, true));
    let r = prepare(&mut s, json!({"file": path.to_string_lossy(), "provider": "gemini"})).unwrap();
    assert!(r["likeness"].as_f64().unwrap() < 0.1, "{r}");
    assert!(texts(&r["warnings"]).iter().any(|w| w.contains("looks less like the photo")), "{r}");
    // A threshold of 0 never warns.
    let r = prepare(&mut s, json!({"file": path.to_string_lossy(), "provider": "gemini", "likenessThreshold": 0})).unwrap();
    assert!(texts(&r["warnings"]).is_empty(), "{r}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn bad_params_and_files_are_errors() {
    let path = photo_file("bad", 64, 64);
    let p = path.to_string_lossy().to_string();
    let mut s = Session::new();
    for bad in [
        json!({}),
        json!({"file": 5}),
        json!({"path": 5}),
        json!({"file": p, "maxSide": 10}),
        json!({"file": p, "maxSide": 5000}),
        json!({"file": p, "maxSide": "big"}),
        json!({"file": p, "likenessThreshold": 2}),
        json!({"file": p, "keepAlpha": "no"}),
        json!({"file": p, "provider": "openai"}),
        json!({"file": p, "provider": 1}),
    ] {
        assert!(prepare(&mut s, bad.clone()).is_err(), "{bad}");
    }
    let text = path.with_file_name("notes.txt");
    std::fs::write(&text, b"not an image").unwrap();
    assert!(prepare(&mut s, json!({"file": text.to_string_lossy()})).is_err());
    assert!(prepare(&mut s, json!({"file": path.with_file_name("missing.png").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_large_result_needs_out() {
    let path = photo_file("large", 2100, 2100);
    let mut s = Session::new();
    let e = prepare(&mut s, json!({"file": path.to_string_lossy(), "maxSide": 4096, "cleanUp": false})).unwrap_err();
    assert!(e.contains("pass path"), "{e}");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A HEIC photo (what a phone makes) is read as File → Open reads it, so Photo to Stitches takes it.
#[cfg(feature = "heif")]
#[test]
fn a_heic_photo_is_read() {
    let dir = std::env::temp_dir().join(format!("vc-photo-{}-heic", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("phone.heic");
    std::fs::write(&path, include_bytes!("../../../tests/fixtures/heif/quadrants.heic")).unwrap();
    let mut s = Session::new();
    let r = s.execute("photo.palette", &json!({"file": path.to_string_lossy()})).unwrap();
    assert!(r["width"].as_u64().unwrap() > 0 && !r["colors"].as_array().unwrap().is_empty(), "{r}");
    // A file that is no image still says what the first reader said.
    let junk = dir.join("junk.heic");
    std::fs::write(&junk, b"not an image").unwrap();
    assert!(s.execute("photo.palette", &json!({"file": junk.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
