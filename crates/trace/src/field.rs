//! Direction field: which way the texture of a picture runs, cell by cell, and how clearly.
//!
//! The method is the gradient structure tensor (the "second-moment matrix") described by Kass and
//! Witkin, *Analyzing oriented patterns* (CVGIP 37, 1987) and by Bigün and Granlund (1987): at each
//! pixel the outer product of the image gradient with itself, `J = [gx², gx·gy; gx·gy, gy²]`, is
//! averaged over a neighbourhood (a Gaussian of the integration scale). The eigenvector of `J`'s
//! larger eigenvalue is the direction across the strokes, so the texture (hair, grain) runs at right
//! angles to it; the coherence `((λ1 − λ2) / (λ1 + λ2))²` is 1 where one direction dominates and 0
//! where the picture is flat or has no single direction. Averaging the tensor, not the angles, is
//! what keeps opposite gradients (both sides of a dark streak) from cancelling.
//!
//! The work is bounded: the picture is box-averaged down until its longest side is at most
//! [`MAX_WORK_SIDE`] pixels, the Gaussians are capped, and the grid has at most [`MAX_CELLS`]
//! cells. Everything runs on one thread with no clock, so it builds for the web as well.

use serde::Serialize;

use crate::{MAX_SOURCE_PIXELS, Raster};

/// The longest side the tensor is computed at; larger pictures are box-averaged down first.
pub const MAX_WORK_SIDE: u32 = 1024;
/// The most cells a field may have.
pub const MAX_CELLS: usize = 1 << 18;
/// The cell sizes taken, in source pixels.
pub const CELL_RANGE: (f64, f64) = (2.0, 512.0);
/// The integration scales taken (the Gaussian's sigma), in source pixels.
pub const SMOOTHING_RANGE: (f64, f64) = (0.5, 256.0);
/// The largest Gaussian sigma used on the working image, in working pixels.
const MAX_WORK_SIGMA: f64 = 48.0;

/// How a direction field is measured. Sizes are in pixels of the source picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldParams {
    /// The side of a grid cell: one direction is reported per cell.
    pub cell: f64,
    /// The integration scale: the sigma of the Gaussian the tensor is averaged with. Larger
    /// follows broader strokes and ignores single hairs.
    pub smoothing: f64,
}

impl Default for FieldParams {
    fn default() -> Self {
        Self { cell: 8.0, smoothing: 6.0 }
    }
}

/// The texture direction of a picture on a grid of cells, row by row from the top-left cell.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectionField {
    /// The source picture's size in pixels.
    pub width: u32,
    pub height: u32,
    /// A cell's side in source pixels; cell `(c, r)` covers `c·cell .. (c+1)·cell` across.
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    /// Per cell: the direction the texture runs, in degrees counter-clockwise from horizontal as
    /// seen (y up on screen), in `[0, 180)`.
    pub angles: Vec<f32>,
    /// Per cell: how clearly one direction dominates, 0–1.
    pub coherence: Vec<f32>,
    /// The size the tensor was computed at (the picture box-averaged down to it, if larger).
    pub worked_at: (u32, u32),
}

impl DirectionField {
    /// The angle (degrees, as [`DirectionField::angles`]) and coherence of cell `(col, row)`.
    pub fn cell_at(&self, col: usize, row: usize) -> Option<(f64, f64)> {
        let i = row.checked_mul(self.cols)?.checked_add(col)?;
        if col >= self.cols {
            return None;
        }
        Some((f64::from(*self.angles.get(i)?), f64::from(*self.coherence.get(i)?)))
    }

    /// The direction at pixel `(x, y)`: the cells around it blended (doubled angles weighted by
    /// coherence, so a direction and its opposite agree) → (angle in degrees `[0, 180)`, strength
    /// 0–1). `None` off the grid or where nothing has a direction.
    pub fn sample(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !(x.is_finite() && y.is_finite()) || self.cols == 0 || self.rows == 0 || !self.cell.is_finite() || self.cell <= 0.0 {
            return None;
        }
        let fx = (x / self.cell - 0.5).clamp(0.0, (self.cols - 1) as f64);
        let fy = (y / self.cell - 0.5).clamp(0.0, (self.rows - 1) as f64);
        let (c0, r0) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - c0 as f64, fy - r0 as f64);
        let mut v = (0.0, 0.0);
        for (dc, dr, w) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let (c, r) = ((c0 + dc).min(self.cols - 1), (r0 + dr).min(self.rows - 1));
            if let Some((a, coh)) = self.cell_at(c, r) {
                let t = (2.0 * a).to_radians();
                v.0 += w * coh * t.cos();
                v.1 += w * coh * t.sin();
            }
        }
        let s = v.0.hypot(v.1);
        if s < 1e-9 {
            return None;
        }
        Some((normal_angle(v.1.atan2(v.0).to_degrees() / 2.0), s.min(1.0)))
    }
}

/// `a` degrees brought into `[0, 180)`.
fn normal_angle(a: f64) -> f64 {
    let r = a.rem_euclid(180.0);
    if r >= 180.0 { 0.0 } else { r }
}

/// The direction field of `img` (its alpha composited onto white). An image whose bytes don't
/// match its size, an empty one, one of more than [`MAX_SOURCE_PIXELS`], settings out of range or a
/// grid of more than [`MAX_CELLS`] cells is an error.
pub fn direction_field(img: &Raster, p: &FieldParams) -> Result<DirectionField, String> {
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 {
        return Err("the image is empty".into());
    }
    let n = (w as usize).checked_mul(h as usize).ok_or("the image is too large")?;
    if n.checked_mul(4) != Some(img.rgba.len()) {
        return Err(format!("{} bytes of RGBA data for a {w} × {h} image", img.rgba.len()));
    }
    if u64::from(w) * u64::from(h) > MAX_SOURCE_PIXELS {
        return Err(format!("the image is {w} × {h} pixels, more than {} megapixels", MAX_SOURCE_PIXELS >> 20));
    }
    let within = |v: f64, (lo, hi): (f64, f64)| v.is_finite() && (lo..=hi).contains(&v);
    if !within(p.cell, CELL_RANGE) {
        return Err(format!("the cell size must be {}–{} pixels, not {}", CELL_RANGE.0, CELL_RANGE.1, p.cell));
    }
    if !within(p.smoothing, SMOOTHING_RANGE) {
        return Err(format!("the smoothing must be {}–{} pixels, not {}", SMOOTHING_RANGE.0, SMOOTHING_RANGE.1, p.smoothing));
    }
    let cols = (f64::from(w) / p.cell).ceil().max(1.0) as usize;
    let rows = (f64::from(h) / p.cell).ceil().max(1.0) as usize;
    if cols.saturating_mul(rows) > MAX_CELLS {
        return Err(format!("a {cols} × {rows} grid has more than {MAX_CELLS} cells: use a larger cell size"));
    }
    // Box-average down to the working size by a whole factor.
    let k = w.max(h).div_ceil(MAX_WORK_SIDE).max(1);
    let (ww, wh) = (w.div_ceil(k) as usize, h.div_ceil(k) as usize);
    let grey = work_grey(img, k, ww, wh);
    let kf = f64::from(k);
    // A light pre-blur against pixel noise, then gradients by central differences.
    let grey = blur(&grey, ww, wh, (1.0 / kf).max(0.5));
    let (mut jxx, mut jxy, mut jyy) = (vec![0f32; ww * wh], vec![0f32; ww * wh], vec![0f32; ww * wh]);
    let at = |x: usize, y: usize| grey.get(y * ww + x).copied().unwrap_or(0.0);
    for y in 0..wh {
        for x in 0..ww {
            let gx = (at((x + 1).min(ww - 1), y) - at(x.saturating_sub(1), y)) * 0.5;
            let gy = (at(x, (y + 1).min(wh - 1)) - at(x, y.saturating_sub(1))) * 0.5;
            let i = y * ww + x;
            if let (Some(a), Some(b), Some(c)) = (jxx.get_mut(i), jxy.get_mut(i), jyy.get_mut(i)) {
                *a = gx * gx;
                *b = gx * gy;
                *c = gy * gy;
            }
        }
    }
    let sigma = p.smoothing / kf;
    jxx = blur(&jxx, ww, wh, sigma);
    jxy = blur(&jxy, ww, wh, sigma);
    jyy = blur(&jyy, ww, wh, sigma);
    let mut angles = Vec::with_capacity(cols * rows);
    let mut coherence = Vec::with_capacity(cols * rows);
    let cw = p.cell / kf;
    for r in 0..rows {
        for c in 0..cols {
            // The working pixels whose centres fall in the cell (at least the one under its centre).
            let x0 = ((c as f64 * cw).floor() as usize).min(ww - 1);
            let y0 = ((r as f64 * cw).floor() as usize).min(wh - 1);
            let x1 = (((c + 1) as f64 * cw).ceil() as usize).clamp(x0 + 1, ww);
            let y1 = (((r + 1) as f64 * cw).ceil() as usize).clamp(y0 + 1, wh);
            let (mut a, mut b, mut d) = (0f64, 0f64, 0f64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y * ww + x;
                    a += f64::from(jxx.get(i).copied().unwrap_or(0.0));
                    b += f64::from(jxy.get(i).copied().unwrap_or(0.0));
                    d += f64::from(jyy.get(i).copied().unwrap_or(0.0));
                }
            }
            let (angle, coh) = orientation(a, b, d);
            angles.push(angle as f32);
            coherence.push(coh as f32);
        }
    }
    Ok(DirectionField { width: w, height: h, cell: p.cell, cols, rows, angles, coherence, worked_at: (ww as u32, wh as u32) })
}

/// The texture direction (degrees counter-clockwise as seen, `[0, 180)`) and coherence of the
/// tensor `[a, b; b, d]` (image coordinates, y down).
fn orientation(a: f64, b: f64, d: f64) -> (f64, f64) {
    let trace = a + d;
    let diff = ((a - d) * (a - d) + 4.0 * b * b).sqrt();
    if !(trace.is_finite() && diff.is_finite()) || trace <= 1e-12 {
        return (0.0, 0.0);
    }
    let coherence = (diff / trace).clamp(0.0, 1.0).powi(2);
    // The gradient's dominant direction, y down; the texture runs at right angles to it.
    let across = 0.5 * (2.0 * b).atan2(a - d);
    let along = across + std::f64::consts::FRAC_PI_2;
    // On screen y runs up: the angle counter-clockwise as seen is the negated y-down angle.
    (normal_angle(-along.to_degrees()), coherence)
}

/// `img`'s grey (Rec. 601 luma, alpha composited onto white), 0–1, averaged over `k × k` blocks.
fn work_grey(img: &Raster, k: u32, ww: usize, wh: usize) -> Vec<f32> {
    let mut sum = vec![0f32; ww * wh];
    let mut count = vec![0u32; ww * wh];
    let w = img.width as usize;
    let k = k as usize;
    for (i, px) in img.rgba.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        let j = (y / k) * ww + x / k;
        let luma = (f32::from(px[0]) * 0.299 + f32::from(px[1]) * 0.587 + f32::from(px[2]) * 0.114) / 255.0;
        let a = f32::from(px[3]) / 255.0;
        if let (Some(s), Some(c)) = (sum.get_mut(j), count.get_mut(j)) {
            *s += luma * a + (1.0 - a);
            *c += 1;
        }
    }
    sum.iter().zip(&count).map(|(s, &c)| if c > 0 { s / c as f32 } else { 1.0 }).collect()
}

/// `src` (`w × h`) blurred by a Gaussian of `sigma` pixels (capped), edges clamped.
fn blur(src: &[f32], w: usize, h: usize, sigma: f64) -> Vec<f32> {
    let sigma = sigma.clamp(0.0, MAX_WORK_SIGMA);
    if sigma < 0.3 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let radius = (sigma * 3.0).ceil() as usize;
    let mut kernel: Vec<f32> = (0..=2 * radius)
        .map(|i| {
            let d = i as f64 - radius as f64;
            (-(d * d) / (2.0 * sigma * sigma)).exp() as f32
        })
        .collect();
    let total: f32 = kernel.iter().sum();
    for v in &mut kernel {
        *v /= total;
    }
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0f32;
                for (i, kv) in kernel.iter().enumerate() {
                    let (sx, sy) =
                        if horizontal { ((x + i).saturating_sub(radius).min(w - 1), y) } else { (x, (y + i).saturating_sub(radius).min(h - 1)) };
                    acc += kv * src.get(sy * w + sx).copied().unwrap_or(0.0);
                }
                if let Some(o) = out.get_mut(y * w + x) {
                    *o = acc;
                }
            }
        }
        out
    };
    pass(&pass(src, true), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stripes running at `angle` degrees (counter-clockwise as seen), `period` pixels apart.
    fn stripes(w: u32, h: u32, angle: f64, period: f64) -> Raster {
        let a = angle.to_radians();
        // Across the stripes, in image coordinates (y down).
        let n = (a.sin(), a.cos());
        Raster::from_fn(w, h, |x, y| {
            let u = f64::from(x) * n.0 + f64::from(y) * n.1;
            let v = (127.5 + 100.0 * (u / period * std::f64::consts::TAU).sin()) as u8;
            [v, v, v, 255]
        })
    }

    fn angle_error(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(180.0);
        d.min(180.0 - d)
    }

    #[test]
    fn stripes_at_known_angles_are_found() {
        for angle in [0.0, 30.0, 45.0, 72.0, 90.0, 120.0, 135.0, 165.0] {
            let f = direction_field(&stripes(96, 80, angle, 7.0), &FieldParams::default()).unwrap();
            assert_eq!((f.cols, f.rows), (12, 10));
            let mut worst: f64 = 0.0;
            for r in 1..f.rows - 1 {
                for c in 1..f.cols - 1 {
                    let (a, coh) = f.cell_at(c, r).unwrap();
                    worst = worst.max(angle_error(a, angle));
                    assert!(coh > 0.8, "{angle}: coherence {coh}");
                }
            }
            assert!(worst < 3.0, "{angle}: off by {worst}°");
            let (a, s) = f.sample(48.0, 40.0).unwrap();
            assert!(angle_error(a, angle) < 3.0 && s > 0.8, "{angle}: sampled {a} {s}");
        }
    }

    #[test]
    fn a_flat_image_has_no_direction() {
        let flat = Raster::from_fn(40, 40, |_, _| [90, 90, 90, 255]);
        let f = direction_field(&flat, &FieldParams::default()).unwrap();
        assert!(f.coherence.iter().all(|&c| c == 0.0));
        assert_eq!(f.sample(10.0, 10.0), None);
    }

    #[test]
    fn large_images_are_worked_at_a_bounded_size() {
        let f = direction_field(&stripes(4096, 40, 60.0, 9.0), &FieldParams { cell: 64.0, smoothing: 24.0 }).unwrap();
        assert_eq!(f.worked_at, (1024, 10));
        assert_eq!((f.cols, f.rows), (64, 1));
    }

    #[test]
    fn bad_input_is_an_error() {
        let img = stripes(20, 20, 0.0, 5.0);
        assert!(direction_field(&img, &FieldParams { cell: 0.0, ..Default::default() }).is_err());
        assert!(direction_field(&img, &FieldParams { smoothing: f64::NAN, ..Default::default() }).is_err());
        assert!(direction_field(&Raster { width: 20, height: 20, rgba: vec![0; 7] }, &FieldParams::default()).is_err());
        assert!(direction_field(&Raster { width: 0, height: 20, rgba: vec![] }, &FieldParams::default()).is_err());
        // 2000 × 2000 cells: refused before any work.
        let big = Raster { width: 4000, height: 4000, rgba: vec![0; 4000 * 4000 * 4] };
        assert!(direction_field(&big, &FieldParams { cell: 2.0, smoothing: 1.0 }).unwrap_err().contains("cells"));
    }

    #[test]
    fn the_field_serialises_for_inspection() {
        let f = direction_field(&stripes(16, 16, 90.0, 4.0), &FieldParams::default()).unwrap();
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["cols"], 2);
        assert_eq!(v["angles"].as_array().unwrap().len(), 4);
        assert_eq!(v["workedAt"], serde_json::json!([16, 16]));
    }
}
