//! Photo stitch, second pass (STI-30): single-colour photo modes.
//!
//! One thread draws the photo's tone: more thread where it is darker.
//!
//! - **Rows** (`fill: rows`): rows across the picture, `spacing` apart, sewn back and forth. Each
//!   row zigzags about its line with an amplitude that grows with darkness (up to just under half
//!   the spacing, so rows never touch), in stitches that shorten as it darkens: light tone is a
//!   straight row of long stitches, dark tone a dense zigzag. This is amplitude-modulated line
//!   shading, as in engraving and in "squiggle" plotter art.
//! - **Meander** (`fill: meander`): one open path that never crosses itself, through the centres of
//!   the cells of an adaptive Hilbert curve (Hilbert 1891): the picture is divided like a quadtree,
//!   darker cells more finely, and the cells are visited in Hilbert order. Consecutive cells always
//!   share an edge, so each stitch stays within the two cells it joins and no two stitches cross;
//!   finer cells put more, shorter stitches where the tone is darker.
//!
//! Tone is read from the picture's lightness, stretched between its 2nd and 98th percentiles.
//! Spec: `docs/superpowers/specs/2026-11-12-embroidery-photo-second-pass.md`.

use vectorcraft_imageprep::Raster;
use vectorcraft_stitch::Pt;

/// The most rows sewn.
const MAX_ROWS: usize = 5_000;
/// The deepest the meander divides a cell.
const MAX_DEPTH: u32 = 12;

/// The tone of a picture: darkness 0–1 per pixel (`None` where transparent), `k` mm per pixel.
pub(super) struct Tone {
    w: usize,
    h: usize,
    k: f64,
    dark: Vec<Option<f64>>,
    /// Summed-area tables of darkness and of opaque pixels, `(w + 1) × (h + 1)`.
    sum: Vec<f64>,
    cnt: Vec<f64>,
}

impl Tone {
    /// The tone of `img` (already softened as wanted), `k` mm per pixel.
    pub(super) fn new(img: &Raster, k: f64) -> Result<Tone, String> {
        let (w, h) = (img.width as usize, img.height as usize);
        if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(img.rgba.len()) || !(k.is_finite() && k > 0.0) {
            return Err("the image's pixels don't fit its size".into());
        }
        let light: Vec<Option<f64>> = img
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, a]| (a >= 128).then(|| 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)))
            .collect();
        let mut hist = [0usize; 256];
        for l in light.iter().flatten() {
            if let Some(c) = hist.get_mut(l.round().clamp(0.0, 255.0) as usize) {
                *c += 1;
            }
        }
        let total: usize = hist.iter().sum();
        if total == 0 {
            return Err("the photo has no opaque area to stitch".into());
        }
        let level = |share: f64| {
            let want = (total as f64 * share).ceil() as usize;
            let mut acc = 0;
            for (v, c) in hist.iter().enumerate() {
                acc += c;
                if acc >= want.max(1) {
                    return v as f64;
                }
            }
            255.0
        };
        let (lo, hi) = (level(0.02), level(0.98));
        let dark: Vec<Option<f64>> =
            light.iter().map(|l| l.map(|l| if hi - lo < 1.0 { 0.5 } else { ((hi - l) / (hi - lo)).clamp(0.0, 1.0) })).collect();
        let (sw, sh) = (w + 1, h + 1);
        let (mut sum, mut cnt) = (vec![0.0; sw * sh], vec![0.0; sw * sh]);
        for y in 0..h {
            let (mut rs, mut rc) = (0.0, 0.0);
            for x in 0..w {
                if let Some(Some(d)) = dark.get(y * w + x) {
                    rs += d;
                    rc += 1.0;
                }
                let (us, uc) = (sum.get(y * sw + x + 1).copied().unwrap_or(0.0), cnt.get(y * sw + x + 1).copied().unwrap_or(0.0));
                if let Some(v) = sum.get_mut((y + 1) * sw + x + 1) {
                    *v = us + rs;
                }
                if let Some(v) = cnt.get_mut((y + 1) * sw + x + 1) {
                    *v = uc + rc;
                }
            }
        }
        Ok(Tone { w, h, k, dark, sum, cnt })
    }

    /// The picture's size, mm.
    pub(super) fn size(&self) -> (f64, f64) {
        (self.w as f64 * self.k, self.h as f64 * self.k)
    }

    /// Whether every pixel is opaque.
    pub(super) fn opaque(&self) -> bool {
        self.dark.iter().all(Option::is_some)
    }

    /// The darkness at `(x, y)` mm, `None` outside the picture or where it is transparent.
    fn at(&self, x: f64, y: f64) -> Option<f64> {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return None;
        }
        let (px, py) = ((x / self.k) as usize, (y / self.k) as usize);
        if px >= self.w || py >= self.h {
            return None;
        }
        self.dark.get(py * self.w + px).copied().flatten()
    }

    /// The mean darkness and the opaque share of the box `lo..hi` mm.
    fn mean(&self, lo: (f64, f64), hi: (f64, f64)) -> (f64, f64) {
        let px = |v: f64, n: usize| ((v / self.k).round().max(0.0) as usize).min(n);
        let (x0, x1) = (px(lo.0, self.w), px(hi.0, self.w));
        let (y0, y1) = (px(lo.1, self.h), px(hi.1, self.h));
        let (x1, y1) = (x1.max((x0 + 1).min(self.w)), y1.max((y0 + 1).min(self.h)));
        let sw = self.w + 1;
        let at = |t: &[f64], x: usize, y: usize| t.get(y * sw + x).copied().unwrap_or(0.0);
        let boxed = |t: &[f64]| at(t, x1, y1) - at(t, x0, y1) - at(t, x1, y0) + at(t, x0, y0);
        let area = ((x1 - x0) * (y1 - y0)) as f64;
        let c = boxed(&self.cnt);
        if c <= 0.0 || area <= 0.0 {
            return (0.0, 0.0);
        }
        (boxed(&self.sum) / c, c / area)
    }
}

/// Rows `spacing` mm apart, back and forth, zigzagging by darkness; stitches from `max_len` (light)
/// down to half the spacing (dark). A transparent stretch ends a run. More than `budget` points is
/// an error.
pub(super) fn rows(t: &Tone, spacing: f64, max_len: f64, budget: usize) -> Result<Vec<Vec<Pt>>, String> {
    let (w, h) = t.size();
    let n = ((h / spacing).floor() as usize).max(1);
    if n > MAX_ROWS {
        return Err(format!("more than {MAX_ROWS} rows: raise spacing"));
    }
    let min_step = (spacing * 0.5).max(0.3);
    let max_step = max_len.max(min_step);
    let mut runs: Vec<Vec<Pt>> = vec![];
    let mut used = 0usize;
    for j in 0..n {
        let y = (j as f64 + 0.5) * spacing;
        let forward = j % 2 == 0;
        let mut run: Vec<Pt> = vec![];
        let mut x = 0.0f64;
        let mut side = 1.0;
        // Each row ends on the picture's edge, so that the next row starts beside it.
        loop {
            let along = x.min(w);
            let at_x = if forward { along } else { w - along };
            let step = match t.at(at_x.min(w - 1e-6).max(0.0), y) {
                Some(d) => {
                    let a = d * spacing * 0.45;
                    if let Some(p) = Pt::from_mm(at_x, y + side * a) {
                        run.push(p);
                    }
                    side = -side;
                    max_step + (min_step - max_step) * d
                }
                None => {
                    if run.len() >= 2 {
                        runs.push(std::mem::take(&mut run));
                    }
                    run.clear();
                    min_step
                }
            };
            used += 1;
            if used > budget {
                return Err("too many stitches".into());
            }
            if along >= w {
                break;
            }
            x += step;
        }
        if run.len() >= 2 {
            runs.push(run);
        }
    }
    Ok(runs)
}

/// The meander: an adaptive Hilbert curve whose cells are no larger than `coarse` mm, and shrink
/// towards `fine` mm as the tone darkens. Transparent cells break it into runs. More than `budget`
/// cells is an error.
pub(super) fn meander(t: &Tone, fine: f64, coarse: f64, budget: usize) -> Result<Vec<Vec<Pt>>, String> {
    let (w, h) = t.size();
    let coarse = coarse.max(fine);
    let mut m = Meander { t, fine, coarse, budget, cells: 0, runs: vec![vec![]] };
    m.cell((0.0, 0.0), (w, 0.0), (0.0, h), 0)?;
    let mut runs = m.runs;
    runs.retain(|r| r.len() >= 2);
    Ok(runs)
}

struct Meander<'a> {
    t: &'a Tone,
    fine: f64,
    coarse: f64,
    budget: usize,
    cells: usize,
    runs: Vec<Vec<Pt>>,
}

impl Meander<'_> {
    /// The cell at `o` spanned by `x` and `y` (mm): its curve starts in the corner at `o` and ends
    /// in the corner at `o + x`.
    fn cell(&mut self, o: (f64, f64), x: (f64, f64), y: (f64, f64), depth: u32) -> Result<(), String> {
        let corners = [o, (o.0 + x.0, o.1 + x.1), (o.0 + y.0, o.1 + y.1), (o.0 + x.0 + y.0, o.1 + x.1 + y.1)];
        let lo = corners.iter().fold((f64::INFINITY, f64::INFINITY), |a, c| (a.0.min(c.0), a.1.min(c.1)));
        let hi = corners.iter().fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |a, c| (a.0.max(c.0), a.1.max(c.1)));
        let side = (hi.0 - lo.0).max(hi.1 - lo.1);
        let (dark, share) = self.t.mean(lo, hi);
        let light = 1.0 - dark;
        let target = self.fine + (self.coarse - self.fine) * light * light;
        if depth < MAX_DEPTH && share > 0.0 && side > target && side / 2.0 >= self.fine * 0.99 {
            let (hx, hy) = ((x.0 / 2.0, x.1 / 2.0), (y.0 / 2.0, y.1 / 2.0));
            self.cell(o, hy, hx, depth + 1)?;
            self.cell((o.0 + hy.0, o.1 + hy.1), hx, hy, depth + 1)?;
            self.cell((o.0 + hx.0 + hy.0, o.1 + hx.1 + hy.1), hx, hy, depth + 1)?;
            self.cell((o.0 + x.0 + hy.0, o.1 + x.1 + hy.1), (-hy.0, -hy.1), (-hx.0, -hx.1), depth + 1)?;
            return Ok(());
        }
        self.cells += 1;
        if self.cells > self.budget {
            return Err("too many stitches".into());
        }
        let c = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
        match (self.t.at(c.0, c.1), Pt::from_mm(c.0, c.1)) {
            (Some(_), Some(p)) => {
                if let Some(r) = self.runs.last_mut() {
                    r.push(p);
                }
            }
            _ => {
                if self.runs.last().is_some_and(|r| !r.is_empty()) {
                    self.runs.push(vec![]);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A left-to-right gradient, black to white, `w × h` pixels at 1 mm each.
    fn gradient(w: u32, h: u32) -> Tone {
        let img = Raster::from_fn(w, h, |x, _| {
            let v = (f64::from(x) / f64::from(w - 1) * 255.0).round() as u8;
            [v, v, v, 255]
        });
        Tone::new(&img, 1.0).unwrap()
    }

    fn crosses(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
        let o = |p: Pt, q: Pt, r: Pt| {
            (i64::from(q.x) - i64::from(p.x)) * (i64::from(r.y) - i64::from(p.y))
                - (i64::from(q.y) - i64::from(p.y)) * (i64::from(r.x) - i64::from(p.x))
        };
        let (d1, d2, d3, d4) = (o(a, b, c), o(a, b, d), o(c, d, a), o(c, d, b));
        (d1 > 0 && d2 < 0 || d1 < 0 && d2 > 0) && (d3 > 0 && d4 < 0 || d3 < 0 && d4 > 0)
    }

    #[test]
    fn a_uniform_hilbert_curve_steps_one_cell_at_a_time() {
        let img = Raster::from_fn(64, 64, |_, _| [0, 0, 0, 255]);
        let t = Tone::new(&img, 1.0).unwrap();
        let runs = meander(&t, 8.0, 8.0, 10_000).unwrap();
        assert_eq!(runs.len(), 1);
        let r = &runs[0];
        assert_eq!(r.len(), 64);
        assert!(r.windows(2).all(|p| (p[0].dist(p[1]) - 80.0).abs() < 1.0), "{r:?}");
    }

    #[test]
    fn the_meander_is_one_path_denser_in_the_dark_that_never_crosses_itself() {
        let t = gradient(96, 64);
        let runs = meander(&t, 1.5, 8.0, 100_000).unwrap();
        assert_eq!(runs.len(), 1, "an opaque picture is one path");
        let r = &runs[0];
        // Denser on the dark (left) side.
        let left = r.iter().filter(|p| p.x < 480).count();
        assert!(left > 2 * (r.len() - left), "{left} of {}", r.len());
        for i in 0..r.len() - 1 {
            for j in i + 2..r.len() - 1 {
                assert!(!crosses(r[i], r[i + 1], r[j], r[j + 1]), "{i} and {j} cross");
            }
        }
        // Consecutive points are never far apart: the longest is between two coarse cells.
        assert!(r.windows(2).all(|p| p[0].dist(p[1]) <= 80.0 * 1.5));
        assert!(meander(&t, 1.5, 8.0, 10).is_err(), "the budget is kept");
    }

    #[test]
    fn rows_zigzag_more_and_stitch_shorter_in_the_dark() {
        let t = gradient(80, 20);
        let runs = rows(&t, 2.0, 4.0, 100_000).unwrap();
        assert_eq!(runs.len(), 10);
        let r = &runs[0];
        // Row 0 runs left to right: the dark end has short stitches and a wide zigzag.
        let first = r[0].dist(r[1]);
        let last = r[r.len() - 3].dist(r[r.len() - 2]);
        assert!(first < last, "{first} {last}");
        let swing = |p: &[Pt]| p.windows(2).map(|q| (q[0].y - q[1].y).abs()).max().unwrap();
        assert!(swing(&r[..6]) > 10 && swing(&r[r.len() - 2..]) <= 2, "{r:?}");
        assert!(rows(&t, 2.0, 4.0, 5).is_err());
        assert!(rows(&t, 0.001, 4.0, 1_000_000).is_err(), "too many rows");
    }

    #[test]
    fn transparency_breaks_the_rows_and_a_clear_picture_has_no_tone() {
        let img = Raster::from_fn(40, 10, |x, _| if (15..25).contains(&x) { [0, 0, 0, 0] } else { [0, 0, 0, 255] });
        let t = Tone::new(&img, 1.0).unwrap();
        assert!(!t.opaque());
        let runs = rows(&t, 2.0, 4.0, 100_000).unwrap();
        assert_eq!(runs.len(), 10, "five rows, two pieces each");
        let clear = Raster::from_fn(4, 4, |_, _| [0, 0, 0, 0]);
        assert!(Tone::new(&clear, 1.0).is_err());
    }
}
