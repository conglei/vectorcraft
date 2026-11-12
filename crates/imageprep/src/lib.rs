//! VectorCraft photo preparation: a photo made ready for the embroidery pipeline (gap E12).
//!
//! An [`ImageProvider`] takes a photo and returns it prepared: in [`Mode::Faithful`], the only mode
//! so far, the subject is kept exactly as it is (no restyle). The [`NoModel`] provider works on this
//! computer alone: it scales the photo down and smooths noise within its tones, edges kept, but
//! cannot cut the subject out. A hosted model (the app layer's Gemini adapter,
//! `vectorcraft-imagemodel`) is another provider; the engine never talks to the network itself.
//!
//! [`likeness`] checks, cheaply and locally, that what a provider returned still looks like the
//! photo it was given (see its limits there).
//!
//! Spec: `docs/superpowers/specs/2026-11-11-embroidery-image-provider-design.md`.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod likeness;

pub use likeness::{COMPARE_SIDE, LIKENESS_THRESHOLD, Likeness, likeness};
pub use vectorcraft_trace::Raster;

/// The name of the [`NoModel`] provider.
pub const NO_MODEL: &str = "nomodel";
/// The longest side a prepared image has unless asked otherwise, in pixels.
pub const DEFAULT_MAX_SIDE: u32 = 1024;
/// The smallest `max_side` a caller may ask for.
pub const MIN_MAX_SIDE: u32 = 64;
/// The largest `max_side` a caller may ask for (16 megapixels for a square photo).
pub const MAX_MAX_SIDE: u32 = 4096;

/// What a provider does to the photo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Keep the subject exactly as it is: cut it out from its background where the provider can,
    /// clean up noise, never restyle. Restyles are a later, opt-in mode.
    #[default]
    Faithful,
}

impl Mode {
    /// The mode's name in commands.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Faithful => "faithful",
        }
    }
}

/// How to prepare a photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrepareOptions {
    pub mode: Mode,
    /// The longest side of the result, in pixels ([`MIN_MAX_SIDE`]..=[`MAX_MAX_SIDE`]); a smaller
    /// photo is not scaled up.
    pub max_side: u32,
    /// Keep the photo's transparency. Off, the result is flattened onto white.
    pub keep_alpha: bool,
    /// Smooth the noise within tones, edges kept ([`vectorcraft_trace::smoothed`]).
    pub clean_up: bool,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self { mode: Mode::Faithful, max_side: DEFAULT_MAX_SIDE, keep_alpha: true, clean_up: true }
    }
}

/// A prepared photo, the provider that made it and what the caller should know about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub image: Raster,
    pub provider: String,
    pub notes: Vec<String>,
}

/// Something that prepares photos: [`NoModel`] here, hosted models at the app layer.
pub trait ImageProvider: Send + Sync {
    /// The provider's name in commands (`nomodel`, `gemini`).
    fn name(&self) -> &str;
    /// Whether [`ImageProvider::prepare`] sends the photo off this computer.
    fn sends_off_machine(&self) -> bool {
        false
    }
    /// Whether the provider can run now; the error says what is missing (a key, say).
    fn ready(&self) -> Result<(), String> {
        Ok(())
    }
    /// `image` prepared with `options`. Errors are plain sentences for the person.
    fn prepare(&self, image: &Raster, options: &PrepareOptions) -> Result<Prepared, String>;
}

/// Preparation on this computer only: scale down, optionally flatten and clean up; no cut-out.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoModel;

impl ImageProvider for NoModel {
    fn name(&self) -> &str {
        NO_MODEL
    }

    fn prepare(&self, image: &Raster, options: &PrepareOptions) -> Result<Prepared, String> {
        let mut out = downscale(image, options.max_side)?;
        let mut notes = vec!["no model: the background was not removed; the subject is as in the photo".to_string()];
        if (out.width, out.height) != (image.width, image.height) {
            notes.push(format!("scaled from {} × {} to {} × {} pixels", image.width, image.height, out.width, out.height));
        }
        if !options.keep_alpha {
            out = flatten(&out, [255, 255, 255]);
        }
        if options.clean_up {
            out = vectorcraft_trace::smoothed(&out);
            notes.push("noise within tones smoothed, edges kept".to_string());
        }
        Ok(Prepared { image: out, provider: NO_MODEL.to_string(), notes })
    }
}

/// An error unless `img` has pixels and exactly `width * height * 4` bytes of them.
pub fn check(img: &Raster) -> Result<(), String> {
    if img.width == 0 || img.height == 0 {
        return Err("the image is empty".into());
    }
    let want = (img.width as usize).checked_mul(img.height as usize).and_then(|n| n.checked_mul(4));
    if want != Some(img.rgba.len()) {
        return Err(format!("{} bytes of pixels for a {} × {} image", img.rgba.len(), img.width, img.height));
    }
    Ok(())
}

/// The size of a `w × h` image whose longest side is at most `max_side`, aspect kept, each side
/// at least 1; an image already that small keeps its size.
pub fn fit_side(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= max_side || long == 0 {
        return (w, h);
    }
    let k = f64::from(max_side) / f64::from(long);
    let side = |v: u32| ((f64::from(v) * k).round() as u32).clamp(1, max_side);
    (side(w), side(h))
}

fn to_image(img: &Raster) -> Result<image::RgbaImage, String> {
    check(img)?;
    image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone()).ok_or_else(|| "the image's pixels don't fit its size".to_string())
}

fn from_image(img: image::RgbaImage) -> Raster {
    let (width, height) = img.dimensions();
    Raster { width, height, rgba: img.into_raw() }
}

/// `img` scaled down so its longest side is at most `max_side` (box filtered, so thin lines and
/// edges survive); a smaller image is returned as it is.
pub fn downscale(img: &Raster, max_side: u32) -> Result<Raster, String> {
    check(img)?;
    let max_side = max_side.clamp(MIN_MAX_SIDE, MAX_MAX_SIDE);
    let (w, h) = fit_side(img.width, img.height, max_side);
    if (w, h) == (img.width, img.height) {
        return Ok(img.clone());
    }
    Ok(from_image(image::imageops::thumbnail(&to_image(img)?, w, h)))
}

/// `img` scaled to exactly `w × h` (a returned image put back at the size that was sent).
pub fn resize_exact(img: &Raster, w: u32, h: u32) -> Result<Raster, String> {
    if w == 0 || h == 0 || w > MAX_MAX_SIDE || h > MAX_MAX_SIDE {
        return Err(format!("can't scale an image to {w} × {h} pixels"));
    }
    if (w, h) == (img.width, img.height) {
        check(img)?;
        return Ok(img.clone());
    }
    Ok(from_image(image::imageops::resize(&to_image(img)?, w, h, image::imageops::FilterType::Triangle)))
}

/// `img` composited onto the opaque colour `rgb`.
pub fn flatten(img: &Raster, rgb: [u8; 3]) -> Raster {
    let mut rgba = img.rgba.clone();
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        for k in 0..3 {
            p[k] = ((u32::from(p[k]) * a + u32::from(rgb[k]) * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
    Raster { width: img.width, height: img.height, rgba }
}

/// `img` as PNG bytes.
pub fn encode_png(img: &Raster) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    to_image(img)?
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| format!("couldn't encode the image as PNG: {e}"))?;
    Ok(out)
}

/// Image bytes (PNG, JPEG, WebP, GIF…) decoded, refused without decoding when the header says
/// they have more than `max_pixels` pixels.
pub fn decode_capped(bytes: &[u8], max_pixels: u64) -> Result<Raster, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| format!("couldn't read the image: {e}"))?;
    let (w, h) = reader.into_dimensions().map_err(|e| format!("couldn't read the image: {e}"))?;
    if w == 0 || h == 0 {
        return Err("the image is empty".into());
    }
    if u64::from(w) * u64::from(h) > max_pixels {
        return Err(format!("the image is {w} × {h} pixels, more than the {} megapixels taken", max_pixels / 1_000_000));
    }
    let mut reader =
        image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| format!("couldn't read the image: {e}"))?;
    let mut limits = image::Limits::default();
    // Room for 16 bits a channel: more fails as a decode error instead of allocating.
    limits.max_alloc = Some(max_pixels.saturating_mul(8));
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("couldn't decode the image: {e}"))?.into_rgba8();
    Ok(from_image(img))
}

/// A made-up photo for tests and trial runs, so none of them needs a real one: a warm brown blob
/// with a darker patch and a light spot (a pet's head, roughly) on a noisy blue-grey background.
/// The same `w`, `h` and `seed` give the same pixels.
pub fn synthetic_photo(w: u32, h: u32, seed: u32) -> Raster {
    let (cx, cy) = (f64::from(w) * 0.5, f64::from(h) * 0.55);
    let r = f64::from(w.min(h)) * 0.3;
    Raster::from_fn(w, h, |x, y| {
        let mut n = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE35);
        n ^= n >> 15;
        n = n.wrapping_mul(0x2C1B_3C6D);
        n ^= n >> 12;
        let noise = (n % 41) as i32 - 20;
        let (dx, dy) = ((f64::from(x) - cx) / r, (f64::from(y) - cy) / (r * 0.85));
        let shade = |v: i32| (v + noise / 3).clamp(0, 255) as u8;
        if dx * dx + dy * dy <= 1.0 {
            let (ex, ey) = (dx - 0.35, dy + 0.25);
            if ex * ex + ey * ey <= 0.04 {
                [shade(245), shade(240), shade(225), 255]
            } else if dy < -0.2 && dx < 0.0 {
                [shade(90), shade(55), shade(30), 255]
            } else {
                [shade(170), shade(110), shade(60), 255]
            }
        } else {
            let v = |base: i32| (base + noise).clamp(0, 255) as u8;
            [v(120), v(135), v(150), 255]
        }
    })
}

/// The fur directions of [`synthetic_fur`], in degrees counter-clockwise from horizontal as seen:
/// the left half's and the right half's.
pub const FUR_ANGLES: (f64, f64) = (30.0, 120.0);

/// The eye of [`synthetic_fur`] for a `w × h` image: centre and radius in pixels.
pub fn synthetic_fur_eye(w: u32, h: u32) -> ((f64, f64), f64) {
    ((f64::from(w) * 0.72, f64::from(h) * 0.38), f64::from(w.min(h)) * 0.09)
}

/// A made-up close-up of fur for tests and trial runs, so none of them needs a real photo: the left
/// half warm brown hair running at [`FUR_ANGLES`]`.0`, the right half grey-brown hair running at
/// [`FUR_ANGLES`]`.1`, each drawn as streaks of value noise along the hair, and a light round eye in
/// the right half. The same `w`, `h` and `seed` give the same pixels.
pub fn synthetic_fur(w: u32, h: u32, seed: u32) -> Raster {
    let (eye, er) = synthetic_fur_eye(w, h);
    let hash = |a: i64, b: u32| -> f64 {
        let mut n = (a as u32).wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE35);
        n ^= n >> 15;
        n = n.wrapping_mul(0x2C1B_3C6D);
        n ^= n >> 12;
        f64::from(n % 2001) / 1000.0 - 1.0
    };
    // Smooth 1-D value noise: random values at whole steps, eased between them.
    let noise = |u: f64, layer: u32| -> f64 {
        let i = u.floor();
        let t = u - i;
        let t = t * t * (3.0 - 2.0 * t);
        let i = i as i64;
        hash(i, layer) * (1.0 - t) + hash(i + 1, layer) * t
    };
    Raster::from_fn(w, h, |x, y| {
        let (fx, fy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        if (fx - eye.0).hypot(fy - eye.1) <= er {
            return [235, 228, 210, 255];
        }
        let left = fx < f64::from(w) * 0.5;
        let a = (if left { FUR_ANGLES.0 } else { FUR_ANGLES.1 }).to_radians();
        // Across the hair (u) and along it (v), image coordinates.
        let u = fx * a.sin() + fy * a.cos();
        let v = fx * a.cos() - fy * a.sin();
        let streak = 0.8 * noise(u / 2.5, 1) + 0.2 * noise(u / 2.5 + v / 60.0, 2);
        let base: [f64; 3] = if left { [168.0, 108.0, 58.0] } else { [98.0, 92.0, 86.0] };
        let k = 1.0 + 0.2 * streak;
        let c = |v: f64| (v * k).round().clamp(0.0, 255.0) as u8;
        [c(base[0]), c(base[1]), c(base[2]), 255]
    })
}

/// The known fur direction of [`synthetic_fur`] at pixel `(x, y)`: `None` in the eye, near it, and
/// within `margin` pixels of the line between the two halves.
pub fn synthetic_fur_angle(w: u32, h: u32, x: f64, y: f64, margin: f64) -> Option<f64> {
    let (eye, er) = synthetic_fur_eye(w, h);
    let mid = f64::from(w) * 0.5;
    if (x - eye.0).hypot(y - eye.1) <= er + margin || (x - mid).abs() < margin || x < 0.0 || y < 0.0 || x > f64::from(w) || y > f64::from(h) {
        return None;
    }
    Some(if x < mid { FUR_ANGLES.0 } else { FUR_ANGLES.1 })
}

#[cfg(test)]
mod tests;
