//! Photo stitch, second pass (STI-30): the eye and nose detail pass.
//!
//! Small features that stand out from their surroundings (an eye, a nose, a highlight) are lost
//! when a photo is reduced to a few thread colours and filled in long rows. They are found as
//! connected areas whose lightness differs from the average of the area around them (a box of
//! about a quarter of the picture's short side) by at least [`CONTRAST`] levels, that are small
//! (at most [`MAX_SHARE`] of the picture) and compact (not a long edge), and each is sewn last, on
//! top of the fills, in the chart thread nearest its own colour: short fill rows across it and a
//! running stitch around its outline.
//! Spec: `docs/superpowers/specs/2026-11-12-embroidery-photo-second-pass.md`.

use vectorcraft_imageprep::Raster;
use vectorcraft_stitch::{DirectionGrid, FeatherEdges, FieldFillParams, Pt, ThreadChart, field_fill};
use vectorcraft_trace::trace_mask;

/// How far (levels of 255) a feature's lightness is from its surroundings' average.
pub(super) const CONTRAST: f64 = 60.0;
/// The largest feature, as a share of the picture's opaque pixels.
const MAX_SHARE: f64 = 0.05;
/// The smallest feature, mm².
const MIN_AREA_MM2: f64 = 1.0;
/// The most features sewn.
const MAX_FEATURES: usize = 16;
/// The stitch length around a feature's outline, mm.
const OUTLINE_STITCH: f64 = 1.2;

/// A detail found: its centre and radius (the farthest outline point from the centre), mm, its
/// thread's index in the chart, and its runs (fill rows, then the outline).
#[derive(Clone, Debug)]
pub(super) struct Feature {
    pub centre: (f64, f64),
    pub radius: f64,
    pub thread: usize,
    pub runs: Vec<Vec<Pt>>,
    pub rings: Vec<Vec<(f64, f64)>>,
}

/// The pixel lightness (Rec. 601) of an RGBA pixel.
fn luma(p: &[u8]) -> f64 {
    match p {
        [r, g, b, ..] => 0.299 * f64::from(*r) + 0.587 * f64::from(*g) + 0.114 * f64::from(*b),
        _ => 0.0,
    }
}

/// The details of `img` (`k` mm per pixel), most contrast first, each matched to `chart` and
/// filled; no more than `budget` stitches in all.
pub(super) fn features(img: &Raster, k: f64, chart: &ThreadChart, seed: u64, budget: usize) -> Result<Vec<Feature>, String> {
    let (w, h) = (img.width as usize, img.height as usize);
    let n = w.checked_mul(h).filter(|&n| n > 0 && n * 4 == img.rgba.len()).ok_or("the image's pixels don't fit its size")?;
    let opaque: Vec<bool> = img.rgba.as_chunks::<4>().0.iter().map(|p| p[3] >= 128).collect();
    let light: Vec<f64> = img.rgba.as_chunks::<4>().0.iter().map(|p| luma(p)).collect();
    // Summed-area tables of the opaque pixels' lightness and count.
    let (sw, sh) = (w + 1, h + 1);
    let mut sum = vec![0.0f64; sw * sh];
    let mut cnt = vec![0.0f64; sw * sh];
    for y in 0..h {
        let (mut row_s, mut row_c) = (0.0, 0.0);
        for x in 0..w {
            let i = y * w + x;
            if opaque.get(i) == Some(&true) {
                row_s += light.get(i).copied().unwrap_or(0.0);
                row_c += 1.0;
            }
            let (up_s, up_c) = (sum.get(y * sw + x + 1).copied().unwrap_or(0.0), cnt.get(y * sw + x + 1).copied().unwrap_or(0.0));
            if let Some(s) = sum.get_mut((y + 1) * sw + x + 1) {
                *s = up_s + row_s;
            }
            if let Some(c) = cnt.get_mut((y + 1) * sw + x + 1) {
                *c = up_c + row_c;
            }
        }
    }
    let at = |t: &[f64], x: usize, y: usize| t.get(y * sw + x).copied().unwrap_or(0.0);
    let boxed = |t: &[f64], x0: usize, y0: usize, x1: usize, y1: usize| at(t, x1, y1) - at(t, x0, y1) - at(t, x1, y0) + at(t, x0, y0);
    let radius = ((w.min(h) as f64) * 0.12).round().max(2.0) as usize;
    let mut mask = vec![false; n];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(radius), (y + radius + 1).min(h));
        for x in 0..w {
            let i = y * w + x;
            if opaque.get(i) != Some(&true) {
                continue;
            }
            let (x0, x1) = (x.saturating_sub(radius), (x + radius + 1).min(w));
            let c = boxed(&cnt, x0, y0, x1, y1);
            if c <= 0.0 {
                continue;
            }
            let mean = boxed(&sum, x0, y0, x1, y1) / c;
            if let (Some(m), Some(l)) = (mask.get_mut(i), light.get(i)) {
                *m = (l - mean).abs() >= CONTRAST;
            }
        }
    }
    let total_opaque = opaque.iter().filter(|o| **o).count().max(1) as f64;
    let min_px = (MIN_AREA_MM2 / (k * k)).max(4.0);
    let mut found: Vec<(f64, vectorcraft_trace::Component)> = vec![];
    for comp in trace_mask(&mask, w, h) {
        let px = comp.pixels as f64;
        if px < min_px || px > MAX_SHARE * total_opaque {
            continue;
        }
        // Compact: its box not much longer one way than the other, and mostly filled.
        let (mut lo, mut hi) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
        for &(x, y) in &comp.outer.pts {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let (bw, bh) = (f64::from(hi.0 - lo.0).max(1.0), f64::from(hi.1 - lo.1).max(1.0));
        if bw.min(bh) / bw.max(bh) < 0.35 || px / (bw * bh) < 0.4 {
            continue;
        }
        found.push((px, comp));
    }
    // The largest first: the eye before a glint in it.
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.truncate(MAX_FEATURES);
    let mut out = vec![];
    let mut used = 0usize;
    for (_, comp) in found {
        // The feature's own colour: the mean of its pixels.
        let mut acc = [0.0f64; 3];
        let mut count = 0.0;
        let ring = |l: &vectorcraft_trace::Loop| l.pts.iter().map(|&(x, y)| (f64::from(x) * k, f64::from(y) * k)).collect::<Vec<_>>();
        let mut rings = vec![ring(&comp.outer)];
        rings.extend(comp.holes.iter().map(ring));
        let outer = rings.first().cloned().unwrap_or_default();
        let (lo, hi) = bounds(&outer);
        let (px0, py0) = ((lo.0 / k).floor().max(0.0) as usize, (lo.1 / k).floor().max(0.0) as usize);
        let (px1, py1) = (((hi.0 / k).ceil() as usize).min(w), ((hi.1 / k).ceil() as usize).min(h));
        for y in py0..py1 {
            for x in px0..px1 {
                let i = y * w + x;
                if super::stitches::winding(&rings, ((x as f64 + 0.5) * k, (y as f64 + 0.5) * k)) != 0
                    && let Some(p) = img.rgba.get(i * 4..i * 4 + 3)
                {
                    for (a, v) in acc.iter_mut().zip(p) {
                        *a += f64::from(*v);
                    }
                    count += 1.0;
                }
            }
        }
        if count <= 0.0 {
            continue;
        }
        let rgb = acc.map(|a| (a / count).round().clamp(0.0, 255.0) as u8);
        let Some(thread) = chart.nearest(rgb) else { continue };
        let centre = ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5);
        let radius = outer.iter().map(|p| (p.0 - centre.0).hypot(p.1 - centre.1)).fold(0.0, f64::max);
        let params = FieldFillParams {
            spacing: 0.35,
            min_length: 1.0,
            max_length: 2.0,
            feather: 0.0,
            edges: FeatherEdges::Both,
            seed: seed ^ out.len() as u64,
            fallback_angle: 45.0,
            max_stitch: 4.0,
            ..FieldFillParams::default()
        };
        let mut runs = field_fill(&rings, vectorcraft_geom::FillRule::NonZero, &DirectionGrid::uniform(45.0), &params, budget.saturating_sub(used))?;
        runs.push(outline(&outer));
        used = used.saturating_add(runs.iter().map(Vec::len).sum());
        if used > budget {
            return Err("the detail pass has too many stitches".into());
        }
        out.push(Feature { centre, radius, thread: thread.index, runs, rings });
    }
    Ok(out)
}

/// The box `ring` lies in.
fn bounds(ring: &[(f64, f64)]) -> ((f64, f64), (f64, f64)) {
    let mut lo = (f64::INFINITY, f64::INFINITY);
    let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in ring {
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0), hi.1.max(p.1));
    }
    if !(lo.0.is_finite() && hi.0.is_finite()) {
        return ((0.0, 0.0), (0.0, 0.0));
    }
    (lo, hi)
}

/// A running stitch once around `ring` (mm), in stitches of about [`OUTLINE_STITCH`], closed.
fn outline(ring: &[(f64, f64)]) -> Vec<Pt> {
    let mut out: Vec<Pt> = vec![];
    let Some(&first) = ring.first() else { return out };
    let mut along = 0.0;
    let mut prev = first;
    out.extend(Pt::from_mm(first.0, first.1));
    for &p in ring.iter().skip(1).chain(std::iter::once(&first)) {
        along += (p.0 - prev.0).hypot(p.1 - prev.1);
        prev = p;
        if along >= OUTLINE_STITCH {
            out.extend(Pt::from_mm(p.0, p.1));
            along = 0.0;
        }
    }
    if out.last() != Pt::from_mm(first.0, first.1).as_ref() {
        out.extend(Pt::from_mm(first.0, first.1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_outline_is_sewn_in_short_stitches_and_closes() {
        let ring: Vec<(f64, f64)> =
            (0..64).map(|i| f64::from(i) / 64.0 * std::f64::consts::TAU).map(|t| (10.0 + 3.0 * t.cos(), 10.0 + 3.0 * t.sin())).collect();
        let run = outline(&ring);
        assert_eq!(run.first(), run.last());
        assert!(run.len() > 10, "{run:?}");
        assert!(run.windows(2).all(|p| p[0].dist(p[1]) <= 20.0), "{run:?}");
        assert!(outline(&[]).is_empty());
    }
}
