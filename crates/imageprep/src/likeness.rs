//! Does a prepared photo still look like the photo it came from?
//!
//! Both are scaled to a small grid ([`COMPARE_SIDE`] pixels on the long side, the original's
//! aspect) and compared only where the prepared image still has its subject: pixels at least half
//! opaque that are not its flat background (a colour that covers most of its border, as a model
//! that cannot return transparency paints it), less the subject's outermost pixel ring, which blends
//! into either image's background. There the score combines:
//!
//! - the mean structural similarity (SSIM: Z. Wang, A. C. Bovik, H. R. Sheikh and E. P.
//!   Simoncelli, "Image Quality Assessment: From Error Visibility to Structural Similarity",
//!   2004) of the luminance over 8 × 8 windows, four pixels apart, weighted 0.8, and
//! - how close the colours are on average (1 at no difference, 0 at a mean RGB distance of
//!   [`COLOUR_SCALE`] levels or more), weighted 0.2.
//!
//! Limits: it is a check, not a judge. At this size fine detail (an eye's colour, a whisker) does
//! not count; a different animal in the same pose and colours can score well; a subject the same
//! colour as a flat background is taken for background; a reframed or zoomed result scores low
//! although nothing changed in the subject. A low score asks a person to look, nothing more.

use crate::{Raster, check, fit_side, flatten, resize_exact};

/// The long side of the grid the two images are compared on, in pixels.
pub const COMPARE_SIDE: u32 = 96;
/// Below this score a prepared photo carries a warning.
pub const LIKENESS_THRESHOLD: f64 = 0.6;
/// The mean RGB distance (levels) at which the colour term reaches 0.
const COLOUR_SCALE: f64 = 120.0;
/// Colours this close (RGB distance, levels) to the border's colour count as flat background.
const FLAT: f64 = 30.0;
/// The share of the subject below which the prepared image counts as having lost it.
const MIN_SUBJECT: f64 = 0.01;
/// The SSIM window and its stride, in grid pixels.
const WINDOW: usize = 8;
const STRIDE: usize = 4;

/// The result of [`likeness`].
#[derive(Clone, Debug, PartialEq)]
pub struct Likeness {
    /// 0 (nothing alike) to 1 (the same).
    pub score: f64,
    /// The share of the grid compared: the prepared image's subject.
    pub subject: f64,
    /// What the caller should know about the comparison.
    pub notes: Vec<String>,
}

impl Likeness {
    fn none(note: &str) -> Self {
        Self { score: 0.0, subject: 0.0, notes: vec![note.to_string()] }
    }
}

/// Rec. 601 luma of an opaque pixel.
fn luma(p: &[u8]) -> f64 {
    match p {
        [r, g, b, ..] => 0.299 * f64::from(*r) + 0.587 * f64::from(*g) + 0.114 * f64::from(*b),
        _ => 0.0,
    }
}

fn dist(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).take(3).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>().sqrt()
}

/// The colour most of the opaque border pixels of `img` are within [`FLAT`] of, if any (75 % or
/// more of them).
fn flat_border(img: &Raster) -> Option<[u8; 3]> {
    let (w, h) = (img.width, img.height);
    let mut border = Vec::new();
    for x in 0..w {
        border.push(img.pixel(x, 0));
        border.push(img.pixel(x, h.saturating_sub(1)));
    }
    for y in 0..h {
        border.push(img.pixel(0, y));
        border.push(img.pixel(w.saturating_sub(1), y));
    }
    border.retain(|p| p[3] >= 128);
    if border.len() < 8 {
        return None;
    }
    let median = |k: usize| {
        let mut v: Vec<u8> = border.iter().map(|p| p[k]).collect();
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    let c = [median(0), median(1), median(2)];
    let near = border.iter().filter(|p| dist(&p[..], &c) <= FLAT).count();
    (near * 4 >= border.len() * 3).then_some(c)
}

/// How much `prepared` still looks like `original` (see the module documentation).
pub fn likeness(original: &Raster, prepared: &Raster) -> Likeness {
    if check(original).is_err() || check(prepared).is_err() {
        return Likeness::none("one of the images is empty");
    }
    let (gw, gh) = fit_side(original.width, original.height, COMPARE_SIDE);
    let mut notes = Vec::new();
    let aspect = |r: &Raster| f64::from(r.width) / f64::from(r.height);
    if (aspect(original) / aspect(prepared) - 1.0).abs() > 0.02 {
        notes.push(format!(
            "the prepared image is {} × {}, not the photo's shape ({} × {}): it was compared stretched to the photo's",
            prepared.width, prepared.height, original.width, original.height
        ));
    }
    let (Ok(a), Ok(b)) = (resize_exact(original, gw, gh), resize_exact(prepared, gw, gh)) else {
        return Likeness::none("the images could not be scaled for comparison");
    };
    let bg = flat_border(&b);
    let a = flatten(&a, [255, 255, 255]);
    let mask: Vec<bool> = b.rgba.as_chunks::<4>().0.iter().map(|p| p[3] >= 128 && bg.is_none_or(|c| dist(&p[..], &c) > FLAT)).collect();
    let b = flatten(&b, [255, 255, 255]);
    let (w, h) = (gw as usize, gh as usize);
    // The subject's outermost ring is a blend of subject and background in the scaled image, and
    // of subject and the photo's own background in the original: left out where something is left.
    let inside = |x: usize, y: usize| mask.get(y * w + x) == Some(&true);
    let eroded: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            inside(x, y)
                && (x == 0 || inside(x - 1, y))
                && (x + 1 >= w || inside(x + 1, y))
                && (y == 0 || inside(x, y - 1))
                && (y + 1 >= h || inside(x, y + 1))
        })
        .collect();
    let subject = mask.iter().filter(|m| **m).count() as f64 / mask.len().max(1) as f64;
    let mask = if eroded.iter().filter(|m| **m).count() as f64 >= MIN_SUBJECT * mask.len() as f64 { eroded } else { mask };
    let count = mask.iter().filter(|m| **m).count();
    if subject < MIN_SUBJECT || count == 0 {
        return Likeness::none("the prepared image has no subject left: it is blank or all background");
    }
    let px = |img: &Raster, i: usize| img.rgba.get(i * 4..i * 4 + 4).unwrap_or(&[0, 0, 0, 0]).to_vec();
    // The colour term over the whole subject.
    let mean_dist = (0..mask.len()).filter(|&i| mask.get(i) == Some(&true)).map(|i| dist(&px(&a, i), &px(&b, i))).sum::<f64>() / count as f64;
    let colour = (1.0 - mean_dist / COLOUR_SCALE).clamp(0.0, 1.0);
    // SSIM over the windows that are at least half subject; the whole subject as one window when
    // none are (a tiny subject).
    let ssim_of = |idx: &[usize]| -> f64 {
        let n = idx.len() as f64;
        let (la, lb): (Vec<f64>, Vec<f64>) = idx.iter().map(|&i| (luma(&px(&a, i)), luma(&px(&b, i)))).unzip();
        let (ma, mb) = (la.iter().sum::<f64>() / n, lb.iter().sum::<f64>() / n);
        let va = la.iter().map(|v| (v - ma).powi(2)).sum::<f64>() / n;
        let vb = lb.iter().map(|v| (v - mb).powi(2)).sum::<f64>() / n;
        let cov = la.iter().zip(&lb).map(|(x, y)| (x - ma) * (y - mb)).sum::<f64>() / n;
        let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
        ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2))
    };
    let mut windows = Vec::new();
    let mut y = 0;
    while y + WINDOW <= h.max(WINDOW) {
        let mut x = 0;
        while x + WINDOW <= w.max(WINDOW) {
            let idx: Vec<usize> = (y..(y + WINDOW).min(h))
                .flat_map(|yy| (x..(x + WINDOW).min(w)).map(move |xx| yy * w + xx))
                .filter(|&i| mask.get(i) == Some(&true))
                .collect();
            if idx.len() * 2 >= WINDOW * WINDOW {
                windows.push(ssim_of(&idx));
            }
            x += STRIDE;
        }
        y += STRIDE;
    }
    let ssim = if windows.is_empty() {
        let all: Vec<usize> = (0..mask.len()).filter(|&i| mask.get(i) == Some(&true)).collect();
        ssim_of(&all)
    } else {
        windows.iter().sum::<f64>() / windows.len() as f64
    };
    let score = (0.8 * ssim.clamp(0.0, 1.0) + 0.2 * colour).clamp(0.0, 1.0);
    if bg.is_some() {
        notes.push(format!("compared on the subject only ({:.0}% of the image; its flat background left out)", subject * 100.0));
    } else if subject < 0.999 {
        notes.push(format!("compared on the subject only ({:.0}% of the image; its transparent background left out)", subject * 100.0));
    }
    Likeness { score: if score.is_finite() { score } else { 0.0 }, subject, notes }
}
