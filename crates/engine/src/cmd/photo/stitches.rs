//! Photo stitch (gap E11): a photo becomes a fill design whose rows follow the way the fur lies.
//!
//! Each stage is a command: `photo.palette` (thread colours), `photo.regions` (editable filled
//! regions with tatami stitch properties), `photo.directionField` (the texture direction) and
//! `photo.toStitches`, which chains them with `photo.prepare`'s providers and opens a document of
//! stitches; `photo.syntheticFur` makes a test picture in code. Nothing leaves this computer unless
//! the provider chosen for `photo.toStitches` does (as for `photo.prepare`).
//! Spec: `docs/superpowers/specs/2026-11-11-embroidery-photo-stitch-design.md`.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, LayerColor, Node, NodeKind, StitchProps, StitchType, Unit};
use vectorcraft_geom::{Affine, FillRule, PathData, PathEl, Rect};
use vectorcraft_imageprep::{FUR_ANGLES, PrepareOptions, Raster, downscale, encode_png, synthetic_fur, synthetic_fur_eye};
use vectorcraft_stitch::{
    Block, DirectionGrid, FeatherEdges, FieldFillParams, PT_PER_MM, Step, StitchPlan, ThreadChart, field_fill, hex, prepare, profile, stats,
    to_document,
};
use vectorcraft_trace::{
    DirectionField, FieldParams, Method, Mode, Palette, TraceParams, TraceResult, continuous_tone, denoise, direction_field, quantize, smoothed,
    trace_within,
};

use super::super::fileio::stitchfile;
use super::super::stitchthreads;
use super::super::{CommandSpec, Result, Session, always, bad, cmd, number_or, text_param};
use super::{MAX_PHOTO_BYTES, chosen_provider, read_photo};
use super::{detail, labels, mono};

const PALETTE: &str = "photo.palette";
const REGIONS: &str = "photo.regions";
const FIELD: &str = "photo.directionField";
const TO_STITCHES: &str = "photo.toStitches";
const SYNTHETIC: &str = "photo.syntheticFur";

/// The longest side a photo is worked at by default, and the range taken (pixels).
const DEFAULT_WORK_SIDE: f64 = 600.0;
const WORK_SIDE: (f64, f64) = (64.0, 2048.0);
/// The thread colours taken.
const COLORS: (f64, f64) = (7.0, 16.0);
/// The most anchor points the regions may have.
const MAX_REGION_ANCHORS: usize = 200_000;
/// The most regions a design may have.
const MAX_REGIONS: usize = 4_000;
/// The most points one region's outline is flattened to.
const MAX_RING_POINTS: usize = 400_000;
/// The default and largest stitch count of `photo.toStitches`.
const DEFAULT_MAX_STITCHES: f64 = 100_000.0;
const MAX_MAX_STITCHES: f64 = 1_000_000.0;
/// The longest stitch the machines take, mm.
const MAX_STITCH_MM: f64 = 12.1;
/// The default and longest gap within a colour joined by travel instead of a trim, mm: one jump
/// record, so the machine never reads it as a trim.
const DEFAULT_TRAVEL_MM: f64 = 3.0;
const MAX_TRAVEL_MM: f64 = 12.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "photo.palette",
            "Photo Thread Palette",
            [],
            None,
            "{file?: (a PNG, JPEG, WebP, GIF, TIFF or BMP file, at most 64 MB) | image?: (the same as base64), colors?: 8 (7–16: the colours the photo is reduced to), chart?: (a thread chart id or name, stitch.threads.list; default the active chart, else the VectorCraft Starter set), maxSide?: 600 (pixels, 64–2048: the photo is worked at this size, never scaled up), blur?: 1.5 (pixels of that size, 0–8: softens fine texture before the colours are chosen), minArea?: 4 (mm², 0–10000: smaller patches join their neighbours), widthMm?: 100 (10–1000), heightMm?: (10–1000; given with widthMm the design fits within both)} the photo reduced to thread colours, each matched to the nearest thread of the chart (CIE76) → {width, height (the worked size, pixels), widthMm, heightMm, chart, colors: [{rgb: \"#rrggbb\" (the photo's colour), coverage (0–1 of the opaque area), regions, brand, code, name, thread: \"#rrggbb\", deltaE}] (one per chart thread, most coverage first)}; works on this computer only; transparent pixels are left out",
            always,
            palette_cmd
        ),
        cmd!(
            "photo.regions",
            "Photo Regions",
            [],
            None,
            "{file?: | image?: (the photo, as photo.palette), …the other params of photo.palette (colors, chart, maxSide, blur, minArea, widthMm, heightMm), mergeGapMm?: 0.5 (mm, 0–3: photo colours matched to one thread are one area, and gaps of another thread this narrow between such areas are closed), minRegionMm2?: 2 (mm², 0–10000: smaller areas are absorbed into the neighbour they share the longest border with), detail?: 40 (0–100: how closely outlines follow the photo; lower is smoother with fewer points), x?: 0, y?: 0 (the top-left corner, document units), angles?: true (each region's tatami angle is its mean direction from photo.directionField; false: 0), cell?: 2, smoothing?: 2 (mm: as photo.directionField)} places the photo's colour regions in the active document (a new one when none is open) as one group of filled paths, a group per thread colour, each filled with its thread's colour and set to sew as tatami (stitch.setType) at its region's direction; the paths are ordinary art to edit; one undo step → {id (the group), colors (as photo.palette), regions, regionsBefore (areas of the photo's colours), merged (areas joined into another of their thread), absorbed (small areas absorbed), document (index of the document)}",
            always,
            regions_cmd
        ),
        cmd!(
            query "photo.directionField",
            "Photo Direction Field",
            [],
            None,
            "{file?: | image?: (the photo, as photo.palette), maxSide?: 600, widthMm?: 100, heightMm?, cell?: 2 (mm, 0.5–50: one direction per cell), smoothing?: 2 (mm, 0.2–100: the neighbourhood the direction is averaged over; larger follows broader strokes), path?: (write a PNG of the field there: the photo dimmed, a line per cell along the direction, as long as it is clear), preview?: false (without path: return that PNG as pngBase64)} which way the photo's texture runs, cell by cell, from the gradient structure tensor (Kass and Witkin 1987) → {width, height (pixels worked at), cell (pixels), cellMm, cols, rows, angles: [degrees counter-clockwise from horizontal as seen, 0–180, row by row from the top left], coherence: [0–1: how clearly one direction dominates], workedAt, path? | pngBase64?}",
            always,
            field_cmd
        ),
        cmd!(
            "photo.toStitches",
            "Photo to Stitches",
            [],
            None,
            "{file?: | image?: (the photo, as photo.palette), provider?: none (use the photo as it is) | nomodel | gemini (as photo.prepare; not given: the photoProvider preference, default nomodel; gemini sends the photo to Google's Gemini API, which cuts the subject out), colors?: 8 (7–16), chart?, maxSide?: 600, blur?: 1.5, minArea?: 4, detail?: 40, widthMm?: 100, heightMm?, mergeGapMm?: 0.5, minRegionMm2?: 2 (as photo.regions), fill?: field (default: rows that follow the photo's direction field) | tatami (straight rows, each region at its mean direction, left as editable regions) | rows (one thread: rows spacing apart that zigzag wider, in shorter stitches, where the photo is darker) | meander (one thread: one open path that never crosses itself through an adaptive Hilbert curve, finer where darker), color?: (rows and meander: the thread as #rrggbb, matched to the chart; default the chart's darkest), spacing?: 0.4 (mm between rows, 0.2–5; rows and meander default 1: the rows' pitch or the meander's finest cell), minLength?: 2, maxLength?: 4 (mm, 0.5–12: each stitch's length is drawn at random between them), feather?: 0 (mm, 0–10: row ends pulled in by a random amount up to this, for a ragged fur edge), featherEdges?: both (default) | start | end (the ends going along the region's direction), seed?: 1 (the same seed sews the same), travelMm?: 3 (mm, 0–12: within a colour, runs are sewn nearest first and a gap up to this is travel, stitched inside the colour's regions or else one jump, not a trim), features?: true (field: small high-contrast features such as eyes and a nose are sewn last over the fills, in the thread nearest their colour: short rows and an outline), cell?: 2, smoothing?: 2 (mm, as photo.directionField), maxStitches?: 100000 (at most 1000000: more is an error)} turns a photo into a stitch design in a new document: prepare (unless none), thread colours (photo.palette), regions (photo.regions), direction field, fills, sewn colour by colour, the colour covering most first. With fill: field the document holds the stitches (needle points, exported unchanged, lock stitches included) over a template layer of the regions; with tatami it holds the regions, sewn by their stitch properties. document.export {format: dst} writes it → {document (index), fill, provider, widthMm, heightMm, colors (as photo.palette), regions, regionsBefore?, merged?, absorbed? (as photo.regions), stitches, jumps, trims, colorChanges, blocks: [{thread, name, stitches, jumps, trims}] (per colour block), details: [{x, y, r}] (mm, the features sewn), warnings}",
            always,
            to_stitches_cmd
        ),
        cmd!(
            query "photo.syntheticFur",
            "Synthetic Fur Picture",
            [],
            None,
            "{path?: (write the PNG there), width?: 480, height?: 360 (pixels, 16–2048), seed?: 1} a picture made in code for trying photo stitch without a photo: the left half warm brown hair at 30°, the right half grey-brown hair at 120°, a light round eye on the right → {width, height, angles: [30, 120] (degrees counter-clockwise as seen), eye: {x, y, r} (pixels), path?}; without path {…, pngBase64}",
            always,
            synthetic_cmd
        ),
    ]
}

/// A number in `p` for `key` (or `default`), which must be finite and within `range`.
fn ranged(p: &Value, key: &str, default: f64, range: (f64, f64), cmd: &str) -> Result<f64> {
    let v = number_or(p, key, default, cmd)?;
    if !(v.is_finite() && (range.0..=range.1).contains(&v)) {
        return Err(bad(cmd, format!("{key} must be {}–{}, not {v}", range.0, range.1)));
    }
    Ok(v)
}

/// The photo `p` gives (`file` or `image`), decoded.
fn photo(p: &Value, cmd: &str) -> Result<Raster> {
    match (text_param(p, "file", cmd)?, text_param(p, "image", cmd)?) {
        (Some(file), None) => read_photo(file, cmd),
        (None, Some(b64)) => {
            if b64.len() > MAX_PHOTO_BYTES / 3 * 4 + 4 {
                return Err(bad(cmd, format!("image is larger than {} MB", MAX_PHOTO_BYTES >> 20)));
            }
            let bytes = vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "image is not base64"))?;
            Raster::decode_to_fit(&bytes).map_err(|e| bad(cmd, format!("image: {e}")))
        }
        (Some(_), Some(_)) => Err(bad(cmd, "pass file or image, not both")),
        (None, None) => Err(bad(cmd, "file (a photo file) or image (the photo as base64) is required")),
    }
}

/// The photo worked at: `p`'s photo scaled to `maxSide`, prepared by a provider when `prepare`
/// (photo.toStitches) and `p` doesn't say `provider: none`. → (image, provider name, warnings)
fn worked(s: &Session, p: &Value, cmd: &str, prepare: bool) -> Result<(Raster, String, Vec<String>)> {
    let side = ranged(p, "maxSide", DEFAULT_WORK_SIDE, WORK_SIDE, cmd)?.round() as u32;
    let use_provider = prepare && text_param(p, "provider", cmd)? != Some("none");
    let chosen = if use_provider { Some(chosen_provider(s, p, cmd)?) } else { None };
    let mut img = photo(p, cmd)?;
    let mut warnings = vec![];
    let mut name = "none".to_string();
    if let Some((provider, w)) = chosen {
        warnings.extend(w);
        // The pipeline softens the picture for regions itself; texture is kept for the field.
        let o = PrepareOptions { max_side: side, keep_alpha: true, clean_up: false, ..Default::default() };
        let prepared = provider.prepare(&img, &o).map_err(|e| bad(cmd, e))?;
        name = prepared.provider;
        img = prepared.image;
    }
    let img = downscale(&img, side).map_err(|e| bad(cmd, e))?;
    Ok((img, name, warnings))
}

/// The design size in mm for a `w × h` picture: `widthMm` (default 100), or within `widthMm` and
/// `heightMm` when both are given, or `heightMm` alone. → millimetres per pixel.
fn mm_per_px(p: &Value, w: u32, h: u32, cmd: &str) -> Result<f64> {
    let range = (10.0, 1000.0);
    let width = match p.get("widthMm") {
        None | Some(Value::Null) => None,
        _ => Some(ranged(p, "widthMm", 100.0, range, cmd)?),
    };
    let height = match p.get("heightMm") {
        None | Some(Value::Null) => None,
        _ => Some(ranged(p, "heightMm", 100.0, range, cmd)?),
    };
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let k = match (width, height) {
        (Some(a), Some(b)) => (a / w).min(b / h),
        (None, Some(b)) => b / h,
        (Some(a), None) => a / w,
        (None, None) => 100.0 / w,
    };
    // A long thin picture at 1000 mm wide could still be very tall: keep both sides in range.
    let k = k.min(range.1 / w).min(range.1 / h);
    if !(k.is_finite() && k > 0.0) {
        return Err(bad(cmd, "the design size must be finite and positive"));
    }
    Ok(k)
}

/// The thread chart `p`'s `chart` names, else the active chart, else the starter set.
fn chart_for(s: &Session, p: &Value, cmd: &str) -> Result<(String, Arc<ThreadChart>)> {
    let found = match text_param(p, "chart", cmd)? {
        Some(key) => Some(stitchthreads::chart(s, key).ok_or_else(|| bad(cmd, format!("no thread chart `{key}` (see stitch.threads.list)")))?),
        None => stitchthreads::active(s),
    };
    let (info, chart) = match found {
        Some(f) => f,
        None => stitchthreads::chart(s, "builtin/starter").ok_or_else(|| bad(cmd, "the starter thread chart is missing"))?,
    };
    Ok((info.name, chart))
}

/// One chart thread's use: its index, pixels, regions, and pixels per photo colour.
type ThreadUse = (usize, usize, usize, Vec<([u8; 3], usize)>);

/// The regions of a worked photo, matched to threads.
struct Regions {
    traced: TraceResult,
    /// Per traced path: the index of its thread in the chart.
    thread_of: Vec<usize>,
    /// Thread indices, most coverage first, with (coverage in pixels, regions, the photo colour
    /// covering most of it, delta E of that colour).
    colors: Vec<(usize, usize, usize, [u8; 3], f64)>,
    chart: Arc<ThreadChart>,
    chart_name: String,
    /// What merging by thread did.
    cleanup: labels::Cleanup,
}

/// `img` blurred by `blur` pixels (none at 0).
fn blurred(img: &Raster, blur: f32, cmd: &str) -> Result<Raster> {
    if blur <= 0.0 {
        return Ok(img.clone());
    }
    let rgba =
        image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone()).ok_or_else(|| bad(cmd, "the image's pixels don't fit its size"))?;
    let b = image::imageops::blur(&rgba, blur);
    Ok(Raster { width: img.width, height: img.height, rgba: b.into_raw() })
}

/// The photo's colour regions: softened by `blur`, reduced to `colors`, small patches merged; then
/// each colour matched to the chart, the pixels relabelled by thread so that what one thread sews
/// is one area, gaps up to `mergeGapMm` closed and areas under `minRegionMm2` absorbed (see
/// `labels`), and the result traced as outlines in pixels.
fn regions(s: &Session, p: &Value, img: &Raster, k: f64, cmd: &str) -> Result<Regions> {
    let colors = ranged(p, "colors", 8.0, COLORS, cmd)?.round() as u32;
    let blur = ranged(p, "blur", 1.5, (0.0, 8.0), cmd)? as f32;
    let min_area = ranged(p, "minArea", 4.0, (0.0, 10_000.0), cmd)?;
    let detail = ranged(p, "detail", 40.0, (0.0, 100.0), cmd)?;
    let gap_mm = ranged(p, "mergeGapMm", 0.5, (0.0, 3.0), cmd)?;
    let min_region = ranged(p, "minRegionMm2", 2.0, (0.0, 10_000.0), cmd)?;
    let (chart_name, chart) = chart_for(s, p, cmd)?;
    let soft = blurred(img, blur, cmd)?;
    let px_of = |mm2: f64| (mm2 / (k * k)).round().clamp(1.0, 1e6) as u32;
    let params = TraceParams {
        mode: Mode::Color,
        palette: Palette::Limited,
        colors,
        paths: detail,
        corners: 50.0,
        noise: px_of(min_area),
        method: Method::Abutting,
        fills: true,
        strokes: false,
        ..TraceParams::default()
    };
    // The photo's colours, as the tracer reduces a photo: smoothed within its tones, quantized, specks merged.
    let (w, h) = (img.width as usize, img.height as usize);
    let toned = if continuous_tone(&soft) { smoothed(&soft) } else { soft.clone() };
    let mut q = quantize(&toned, &params);
    if q.labels.len() != w * h || q.palette.is_empty() {
        return Err(bad(cmd, "the photo has no opaque area to stitch"));
    }
    denoise(&mut q.labels, w, h, params.noise as usize);
    // Each photo colour's thread, and per chart thread: pixels, (pixels per photo colour).
    let mut thread_index: Vec<usize> = vec![];
    let mut thread_of_colour: Vec<u16> = Vec::with_capacity(q.palette.len());
    for &c in &q.palette {
        let m = chart.nearest(c).ok_or_else(|| bad(cmd, "the thread chart is empty"))?;
        let slot = match thread_index.iter().position(|&t| t == m.index) {
            Some(i) => i,
            None => {
                thread_index.push(m.index);
                thread_index.len() - 1
            }
        };
        thread_of_colour.push(u16::try_from(slot).map_err(|_| bad(cmd, "too many thread colours"))?);
    }
    let gap = (gap_mm / k).round().clamp(0.0, labels::MAX_GAP_PX as f64) as usize;
    let (by_thread_labels, cleanup) = labels::by_thread(&q.labels, w, h, &thread_of_colour, gap, px_of(min_region) as usize);
    // The cleaned areas painted in their threads' colours and traced: flat colours, one per thread.
    let mut flat = Raster { width: img.width, height: img.height, rgba: vec![0; w * h * 4] };
    let mut used = vec![false; thread_index.len()];
    for (px, &l) in flat.rgba.as_chunks_mut::<4>().0.iter_mut().zip(&by_thread_labels) {
        let Some(rgb) = thread_index.get(l as usize).and_then(|&t| chart.threads.get(t)).map(|t| t.rgb) else { continue };
        if let Some(u) = used.get_mut(l as usize) {
            *u = true;
        }
        px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    let flat_params = TraceParams { colors: (used.iter().filter(|u| **u).count() as u32).max(2), noise: 1, ..params };
    let traced =
        trace_within(&flat, &flat_params, MAX_REGION_ANCHORS).map_err(|e| bad(cmd, format!("{e}: raise minArea or lower detail or colors")))?;
    if traced.paths.is_empty() {
        return Err(bad(cmd, "the photo has no opaque area to stitch"));
    }
    if traced.paths.len() > MAX_REGIONS {
        return Err(bad(
            cmd,
            format!("the photo makes {} regions, more than {MAX_REGIONS}: raise minArea or blur, or lower colors", traced.paths.len()),
        ));
    }
    let mut thread_of = Vec::with_capacity(traced.paths.len());
    // Per chart thread: pixels, regions, (pixels per photo colour).
    let mut by_thread: Vec<ThreadUse> = thread_index.iter().map(|&t| (t, 0, 0, vec![])).collect();
    for (&l, &photo_l) in by_thread_labels.iter().zip(&q.labels) {
        let (Some(e), Some(&c)) = (by_thread.get_mut(l as usize), q.palette.get(photo_l as usize)) else { continue };
        e.1 += 1;
        match e.3.iter_mut().find(|x| x.0 == c) {
            Some(x) => x.1 += 1,
            None => e.3.push((c, 1)),
        }
    }
    for tp in &traced.paths {
        let m = chart.nearest(tp.color).ok_or_else(|| bad(cmd, "the thread chart is empty"))?;
        thread_of.push(m.index);
        if let Some(e) = by_thread.iter_mut().find(|e| e.0 == m.index) {
            e.2 += 1;
        }
    }
    by_thread.retain(|e| e.2 > 0);
    by_thread.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let colors = by_thread
        .into_iter()
        .map(|(t, px, n, cs)| {
            let main = cs.iter().max_by_key(|c| c.1).map(|c| c.0).unwrap_or([0, 0, 0]);
            let de = chart.threads.get(t).map_or(0.0, |th| vectorcraft_stitch::delta_e(main, th.rgb));
            (t, px, n, main, de)
        })
        .collect();
    Ok(Regions { traced, thread_of, colors, chart, chart_name, cleanup })
}

impl Regions {
    fn colors_json(&self) -> Vec<Value> {
        let total: usize = self.colors.iter().map(|c| c.1).sum();
        self.colors
            .iter()
            .map(|&(t, px, n, rgb, de)| {
                let mut v = json!({
                    "rgb": hex(rgb),
                    "coverage": if total > 0 { (px as f64 / total as f64 * 1000.0).round() / 1000.0 } else { 0.0 },
                    "regions": n,
                });
                if let Some(th) = self.chart.threads.get(t) {
                    v["brand"] = json!(th.brand);
                    v["code"] = json!(th.code);
                    v["name"] = json!(th.name);
                    v["thread"] = json!(hex(th.rgb));
                    v["deltaE"] = json!((de * 100.0).round() / 100.0);
                }
                v
            })
            .collect()
    }

    /// The traced paths' indices in sewing order: colour by colour, most coverage first, each
    /// colour's regions in the order traced (bottom first).
    fn sewing_order(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.thread_of.len());
        for &(t, ..) in &self.colors {
            out.extend(self.thread_of.iter().enumerate().filter(|(_, x)| **x == t).map(|(i, _)| i));
        }
        out
    }

    fn rgb(&self, i: usize) -> [u8; 3] {
        self.thread_of.get(i).and_then(|&t| self.chart.threads.get(t)).map_or([0, 0, 0], |t| t.rgb)
    }
}

/// The direction field of `img` with the cell and smoothing `p` gives in mm (`k` mm per pixel).
fn field(p: &Value, img: &Raster, k: f64, cmd: &str) -> Result<DirectionField> {
    let cell = ranged(p, "cell", 2.0, (0.5, 50.0), cmd)?;
    let smoothing = ranged(p, "smoothing", 2.0, (0.2, 100.0), cmd)?;
    let cell_px = (cell / k).clamp(vectorcraft_trace::CELL_RANGE.0, vectorcraft_trace::CELL_RANGE.1);
    let smoothing_px = (smoothing / k).clamp(vectorcraft_trace::SMOOTHING_RANGE.0, vectorcraft_trace::SMOOTHING_RANGE.1);
    direction_field(img, &FieldParams { cell: cell_px, smoothing: smoothing_px }).map_err(|e| bad(cmd, e))
}

/// `f` in millimetres (`k` mm per pixel, the picture's top-left at the origin).
fn grid_mm(f: &DirectionField, k: f64) -> DirectionGrid {
    let cell = f.cell * k;
    DirectionGrid {
        origin: (cell * 0.5, cell * 0.5),
        cell,
        cols: f.cols,
        rows: f.rows,
        angles: f.angles.iter().map(|&a| f64::from(a)).collect(),
        weights: f.coherence.iter().map(|&c| f64::from(c)).collect(),
    }
}

/// The closed outlines of `path` (pixels) flattened to rings in mm (`k` mm per pixel).
fn rings_mm(path: &PathData, k: f64, cmd: &str) -> Result<Vec<Vec<(f64, f64)>>> {
    let mut rings: Vec<Vec<(f64, f64)>> = vec![];
    let mut count = 0usize;
    let mut over = false;
    vectorcraft_geom::flatten(path.to_bezpath().elements().iter().copied(), 0.2, |el| {
        let q = match el {
            PathEl::MoveTo(q) => {
                rings.push(vec![]);
                q
            }
            PathEl::LineTo(q) => q,
            _ => return,
        };
        count += 1;
        if count > MAX_RING_POINTS {
            over = true;
            return;
        }
        if let Some(r) = rings.last_mut() {
            r.push((q.x * k, q.y * k));
        }
    });
    if over {
        return Err(bad(cmd, "a region's outline has too many points: lower detail"));
    }
    rings.retain(|r| r.len() >= 3);
    Ok(rings)
}

/// A filled path's appearance: `rgb`, no stroke.
fn filled(rgb: [u8; 3]) -> Appearance {
    Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2]))))], ..Default::default() }
}

/// The regions as a group of groups (one per thread colour) of filled paths in document points,
/// the top-left at `at`; `props(i, rings)` gives path `i`'s stitch properties.
fn region_group(d: &mut Document, r: &Regions, k: f64, at: (f64, f64), props: &dyn Fn(usize) -> Option<StitchProps>) -> Node {
    let xf = Affine::translate(at) * Affine::scale(k * PT_PER_MM);
    let mut groups: Vec<Arc<Node>> = vec![];
    for (ci, &(t, ..)) in r.colors.iter().enumerate() {
        let mut paths: Vec<Arc<Node>> = vec![];
        for (i, tp) in r.traced.paths.iter().enumerate() {
            if r.thread_of.get(i) != Some(&t) {
                continue;
            }
            let mut n = Node::path(d.alloc_id(), tp.path.transformed(xf), filled(r.rgb(i)));
            if let Some(sp) = props(i) {
                n.set_stitch_props(sp);
            }
            paths.push(Arc::new(n));
        }
        let mut g = Node::group(d.alloc_id(), paths);
        let label = r.chart.threads.get(t).map(|th| format!("{} {}", th.code, th.name).trim().to_string()).unwrap_or_default();
        g.name = Some(if label.is_empty() { format!("Colour {}", ci + 1) } else { format!("Colour {} · {label}", ci + 1) });
        groups.push(Arc::new(g));
    }
    let mut g = Node::group(d.alloc_id(), groups);
    g.name = Some("Photo regions".into());
    g
}

/// The tatami stitch properties of region `i`: its mean direction (or 0), spacing and length.
fn tatami_props(r: &Regions, i: usize, grid: Option<&DirectionGrid>, k: f64, spacing: Option<f64>, length: Option<f64>) -> StitchProps {
    let angle = grid.and_then(|g| {
        let tp = r.traced.paths.get(i)?;
        let rings = rings_mm(&tp.path, k, REGIONS).ok()?;
        g.mean_angle(&rings, FillRule::NonZero)
    });
    StitchProps {
        kind: StitchType::Tatami,
        fill_angle: Some((angle.unwrap_or(0.0) * 10.0).round() / 10.0),
        fill_spacing: spacing,
        fill_stitch_length: length,
        ..Default::default()
    }
}

fn palette_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (img, _, warnings) = worked(s, p, PALETTE, false)?;
    let k = mm_per_px(p, img.width, img.height, PALETTE)?;
    let r = regions(s, p, &img, k, PALETTE)?;
    Ok(json!({
        "width": img.width,
        "height": img.height,
        "widthMm": round2(f64::from(img.width) * k),
        "heightMm": round2(f64::from(img.height) * k),
        "chart": r.chart_name,
        "colors": r.colors_json(),
        "warnings": warnings,
    }))
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn regions_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let x = number_or(p, "x", 0.0, REGIONS)?;
    let y = number_or(p, "y", 0.0, REGIONS)?;
    if !(x.is_finite() && y.is_finite() && x.abs() <= crate::MAX_COORD && y.abs() <= crate::MAX_COORD) {
        return Err(bad(REGIONS, "x and y must be finite document coordinates"));
    }
    let angles = super::super::switch_or(p, "angles", true, REGIONS)?;
    let (img, _, warnings) = worked(s, p, REGIONS, false)?;
    let k = mm_per_px(p, img.width, img.height, REGIONS)?;
    let r = regions(s, p, &img, k, REGIONS)?;
    let grid = if angles { Some(grid_mm(&field(p, &img, k, REGIONS)?, k)) } else { None };
    let props: Vec<StitchProps> = (0..r.traced.paths.len()).map(|i| tatami_props(&r, i, grid.as_ref(), k, None, None)).collect();
    if s.doc().is_err() {
        let doc = Document::new(f64::from(img.width) * k * PT_PER_MM, f64::from(img.height) * k * PT_PER_MM);
        s.add_document(doc, None);
    }
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Photo Regions", |d, sel| {
        let g = region_group(d, &r, k, (x, y), &|i| props.get(i).copied());
        let gid = g.id;
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({
        "id": id.0,
        "colors": r.colors_json(),
        "regions": r.traced.paths.len(),
        "regionsBefore": r.cleanup.before,
        "merged": r.cleanup.merged,
        "absorbed": r.cleanup.absorbed,
        "document": s.active_index(),
        "warnings": warnings,
    }))
}

fn field_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let out_path = text_param(p, "path", FIELD)?;
    let preview = super::super::switch_or(p, "preview", false, FIELD)?;
    let (img, _, _) = worked(s, p, FIELD, false)?;
    let k = mm_per_px(p, img.width, img.height, FIELD)?;
    let f = field(p, &img, k, FIELD)?;
    let mut v = json!({
        "width": f.width,
        "height": f.height,
        "cell": round2(f.cell),
        "cellMm": round2(f.cell * k),
        "cols": f.cols,
        "rows": f.rows,
        "angles": f.angles.iter().map(|a| (f64::from(*a) * 10.0).round() / 10.0).collect::<Vec<_>>(),
        "coherence": f.coherence.iter().map(|c| (f64::from(*c) * 100.0).round() / 100.0).collect::<Vec<_>>(),
        "workedAt": [f.worked_at.0, f.worked_at.1],
    });
    if out_path.is_some() || preview {
        let png = encode_png(&field_picture(&img, &f)).map_err(|e| bad(FIELD, e))?;
        match out_path {
            Some(o) => {
                super::super::fileio::write_file(o, &png)?;
                v["path"] = json!(o);
            }
            None => v["pngBase64"] = json!(vectorcraft_format::base64_encode(&png)),
        }
    }
    Ok(v)
}

/// `img` dimmed with a line per cell of `f` along its direction, as long as it is clear.
fn field_picture(img: &Raster, f: &DirectionField) -> Raster {
    let mut out = Raster::from_fn(img.width, img.height, |x, y| {
        let [r, g, b, a] = img.pixel(x, y);
        let grey = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
        let v = ((grey * u32::from(a) + 255 * (255 - u32::from(a))) / 255 / 3 + 150) as u8;
        [v, v, v, 255]
    });
    let (w, h) = (out.width as usize, out.height as usize);
    for r in 0..f.rows {
        for c in 0..f.cols {
            let Some((angle, coh)) = f.cell_at(c, r) else { continue };
            let centre = ((c as f64 + 0.5) * f.cell, (r as f64 + 0.5) * f.cell);
            let half = f.cell * 0.45 * coh.max(0.15);
            let t = angle.to_radians();
            let d = (t.cos() * half, -t.sin() * half);
            let steps = (half * 2.0).ceil().clamp(1.0, 4096.0) as usize;
            for i in 0..=steps {
                let s = i as f64 / steps as f64 * 2.0 - 1.0;
                let (x, y) = (centre.0 + d.0 * s, centre.1 + d.1 * s);
                if x < 0.0 || y < 0.0 {
                    continue;
                }
                let (x, y) = (x as usize, y as usize);
                if x < w
                    && y < h
                    && let Some(px) = out.rgba.get_mut((y * w + x) * 4..(y * w + x) * 4 + 4)
                {
                    px.copy_from_slice(&[200, 30, 40, 255]);
                }
            }
        }
    }
    out
}

/// How `photo.toStitches` fills the photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// Colour regions, rows following the direction field.
    Field,
    /// Colour regions left as editable tatami regions.
    Tatami,
    /// One colour: rows zigzagging by tone.
    Rows,
    /// One colour: an open meander, finer where darker.
    Meander,
}

impl Fill {
    fn name(self) -> &'static str {
        match self {
            Fill::Field => "field",
            Fill::Tatami => "tatami",
            Fill::Rows => "rows",
            Fill::Meander => "meander",
        }
    }

    /// The default `spacing`, mm: between rows, or the meander's finest cell.
    fn spacing(self) -> f64 {
        match self {
            Fill::Field | Fill::Tatami => 0.4,
            Fill::Rows | Fill::Meander => 1.0,
        }
    }
}

/// The fill settings of photo.toStitches for `fill`.
fn fill_params(p: &Value, fill: Fill) -> Result<FieldFillParams> {
    let c = TO_STITCHES;
    let spacing = ranged(p, "spacing", fill.spacing(), (0.2, 5.0), c)?;
    let min = ranged(p, "minLength", 2.0, (0.5, 12.0), c)?;
    let max = ranged(p, "maxLength", 4.0, (0.5, 12.0), c)?;
    if max < min {
        return Err(bad(c, format!("maxLength ({max}) must be at least minLength ({min})")));
    }
    let feather = ranged(p, "feather", 0.0, (0.0, 10.0), c)?;
    let edges = match text_param(p, "featherEdges", c)? {
        None | Some("both") => FeatherEdges::Both,
        Some("start") => FeatherEdges::Start,
        Some("end") => FeatherEdges::End,
        Some(o) => return Err(bad(c, format!("featherEdges `{o}`: both, start or end"))),
    };
    let seed = ranged(p, "seed", 1.0, (0.0, 9_007_199_254_740_991.0), c)?.round() as u64;
    Ok(FieldFillParams { spacing, min_length: min, max_length: max, feather, edges, seed, max_stitch: MAX_STITCH_MM, ..FieldFillParams::default() })
}

/// `#rrggbb` (or `rrggbb`) as a colour.
fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The thread a single-colour fill sews in: the chart thread nearest `color`, else the chart's darkest.
fn mono_thread(p: &Value, chart: &ThreadChart, cmd: &str) -> Result<usize> {
    match text_param(p, "color", cmd)? {
        Some(t) => {
            let rgb = parse_rgb(t).ok_or_else(|| bad(cmd, format!("color `{t}`: a colour as #rrggbb")))?;
            chart.nearest(rgb).map(|m| m.index).ok_or_else(|| bad(cmd, "the thread chart is empty"))
        }
        None => {
            let light = |c: [u8; 3]| u32::from(c[0]) * 299 + u32::from(c[1]) * 587 + u32::from(c[2]) * 114;
            chart.threads.iter().enumerate().min_by_key(|(_, t)| light(t.rgb)).map(|(i, _)| i).ok_or_else(|| bad(cmd, "the thread chart is empty"))
        }
    }
}

/// The settings of a single-colour fill.
struct MonoSettings<'a> {
    fill: Fill,
    fp: &'a FieldFillParams,
    max: usize,
    travel: f64,
}

/// A single-colour fill of `img` (`k` mm per pixel): the plan, the chart's name, and the colour
/// entry of the result.
fn sew_mono(s: &Session, p: &Value, img: &Raster, k: f64, m: &MonoSettings) -> Result<(StitchPlan, String, Value)> {
    let c = TO_STITCHES;
    let blur = ranged(p, "blur", 1.5, (0.0, 8.0), c)? as f32;
    let (chart_name, chart) = chart_for(s, p, c)?;
    let t = mono_thread(p, &chart, c)?;
    let thread = chart.threads.get(t).ok_or_else(|| bad(c, "the thread chart is empty"))?;
    let tone = mono::Tone::new(&blurred(img, blur, c)?, k).map_err(|e| bad(c, e))?;
    let max = m.max;
    let too_many = |e: String| bad(c, format!("{e}: the design has more than {max} stitches; raise spacing, make it smaller, or raise maxStitches"));
    let runs = match m.fill {
        Fill::Meander => mono::meander(&tone, m.fp.spacing, m.fp.max_length.max(m.fp.spacing), max).map_err(too_many)?,
        _ => mono::rows(&tone, m.fp.spacing, m.fp.max_length, max).map_err(too_many)?,
    };
    // Travel is stitched only across an opaque picture: a cut-out's background is never sewn.
    let (w, h) = tone.size();
    // The picture with a little room round it: rows end on its edge, and travel runs along it.
    let (lo, w, h) = (-0.5, w + 0.5, h + 0.5);
    let rings = if tone.opaque() { vec![vec![(lo, lo), (w, lo), (w, h), (lo, h)]] } else { vec![] };
    let steps = join_runs(runs, &rings, m.travel, m.fp.max_length);
    if steps.is_empty() {
        return Err(bad(c, "the photo is too small to fill: make the design larger"));
    }
    let plan = StitchPlan { name: "photo".into(), blocks: vec![Block { thread: thread.thread(), steps }] };
    plan.check().map_err(|e| bad(c, e))?;
    let color = json!({
        "rgb": hex(thread.rgb), "coverage": 1.0, "regions": 0, "brand": thread.brand, "code": thread.code, "name": thread.name,
        "thread": hex(thread.rgb), "deltaE": 0.0,
    });
    Ok((plan, chart_name, color))
}

/// The plan prepared for the machine, as a document of needle points.
fn prepared_document(plan: &StitchPlan, warnings: &mut Vec<String>) -> Result<Document> {
    let c = TO_STITCHES;
    let machine = profile("commercial15").ok_or_else(|| bad(c, "the machine profile is missing"))?;
    let prepared = prepare(plan, &machine, &vectorcraft_stitch::PrepareOptions { locks: true, travel_trims: true }).map_err(|e| bad(c, e))?;
    warnings.extend(prepared.warnings);
    Ok(to_document(&prepared.plan))
}

fn to_stitches_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let c = TO_STITCHES;
    let fill = match text_param(p, "fill", c)? {
        None | Some("field") => Fill::Field,
        Some("tatami") => Fill::Tatami,
        Some("rows") => Fill::Rows,
        Some("meander") => Fill::Meander,
        Some(o) => return Err(bad(c, format!("fill `{o}`: field, tatami, rows or meander"))),
    };
    let fp = fill_params(p, fill)?;
    let travel = ranged(p, "travelMm", DEFAULT_TRAVEL_MM, (0.0, MAX_TRAVEL_MM), c)?;
    let with_features = super::super::switch_or(p, "features", true, c)?;
    let mut details: Vec<Value> = vec![];
    let max_stitches = ranged(p, "maxStitches", DEFAULT_MAX_STITCHES, (1.0, MAX_MAX_STITCHES), c)?.round() as usize;
    let (img, provider, mut warnings) = worked(s, p, c, true)?;
    let k = mm_per_px(p, img.width, img.height, c)?;
    let size = (f64::from(img.width) * k, f64::from(img.height) * k);
    if matches!(fill, Fill::Rows | Fill::Meander) {
        let settings = MonoSettings { fill, fp: &fp, max: max_stitches, travel };
        let (plan, chart, color) = sew_mono(s, p, &img, k, &settings)?;
        let doc = prepared_document(&plan, &mut warnings)?;
        let mut out = finish(s, doc, size, max_stitches, &mut warnings)?;
        out["fill"] = json!(fill.name());
        out["provider"] = json!(provider);
        out["chart"] = json!(chart);
        out["colors"] = json!([color]);
        out["regions"] = json!(0);
        out["details"] = json!([]);
        out["warnings"] = json!(warnings);
        return Ok(out);
    }
    let r = regions(s, p, &img, k, c)?;
    let grid = grid_mm(&field(p, &img, k, c)?, k);
    let doc = if fill == Fill::Tatami {
        let mut d = Document::new(1.0, 1.0);
        let length = Some(((fp.min_length + fp.max_length) * 0.5).clamp(1.0, 8.0));
        let props: Vec<StitchProps> = (0..r.traced.paths.len()).map(|i| tatami_props(&r, i, Some(&grid), k, Some(fp.spacing), length)).collect();
        let g = region_group(&mut d, &r, k, (0.0, 0.0), &|i| props.get(i).copied());
        let mut layer = Node::layer(d.alloc_id(), "Photo regions", LayerColor::Preset(0));
        if let Some(ch) = layer.children_mut() {
            ch.push(Arc::new(g));
        }
        d.layers = vec![Arc::new(layer)];
        d
    } else {
        let mut plan = sew_regions(&r, &grid, &fp, k, max_stitches, travel)?;
        if with_features {
            let sewn: usize = plan.blocks.iter().map(|b| b.steps.len()).sum();
            let found = detail::features(&img, k, &r.chart, fp.seed, max_stitches.saturating_sub(sewn)).map_err(|e| bad(c, e))?;
            details = found.iter().map(|f| json!({"x": round2(f.centre.0), "y": round2(f.centre.1), "r": round2(f.radius)})).collect();
            add_features(&mut plan, found, &r.chart, travel);
            plan.check().map_err(|e| bad(c, e))?;
        }
        let mut d = prepared_document(&plan, &mut warnings)?;
        // The regions under the stitches, on a template layer: shown dimmed, never sewn.
        let g = region_group(&mut d, &r, k, (0.0, 0.0), &|_| None);
        let mut layer = Node::layer(d.alloc_id(), "Photo regions", LayerColor::Preset(1));
        if let NodeKind::Layer { template, printable, children, .. } = &mut layer.kind {
            *template = true;
            *printable = false;
            children.push(Arc::new(g));
        }
        d.layers.insert(0, Arc::new(layer));
        d
    };
    let mut out = finish(s, doc, size, max_stitches, &mut warnings)?;
    out["fill"] = json!(fill.name());
    out["provider"] = json!(provider);
    out["chart"] = json!(r.chart_name);
    out["colors"] = json!(r.colors_json());
    out["regions"] = json!(r.traced.paths.len());
    out["regionsBefore"] = json!(r.cleanup.before);
    out["merged"] = json!(r.cleanup.merged);
    out["absorbed"] = json!(r.cleanup.absorbed);
    out["details"] = json!(details);
    out["warnings"] = json!(warnings);
    Ok(out)
}

/// The design `doc` (`size` mm) given its artboard and opened, once the plan its export sews has
/// at most `max_stitches` stitches. → the result's counts.
fn finish(s: &mut Session, mut doc: Document, size: (f64, f64), max_stitches: usize, warnings: &mut Vec<String>) -> Result<Value> {
    let c = TO_STITCHES;
    doc.units = Unit::Millimeters;
    doc.title = "Photo stitches".into();
    let margin = 10.0 * PT_PER_MM;
    let board = Rect::new(0.0, 0.0, size.0 * PT_PER_MM, size.1 * PT_PER_MM).inflate(margin, margin);
    if let Some(a) = doc.artboards.first_mut() {
        a.rect = board;
    }
    let (plan, plan_warnings) = stitchfile::plan(c, &doc, &json!({}), "dst", None)?;
    let st = stats(&plan);
    if st.stitches > max_stitches {
        return Err(bad(
            c,
            format!("the design has {} stitches, more than maxStitches ({max_stitches}): raise spacing, or make it smaller", st.stitches),
        ));
    }
    warnings.extend(plan_warnings);
    s.add_document(doc, None);
    Ok(json!({
        "document": s.active_index(),
        "widthMm": round2(size.0),
        "heightMm": round2(size.1),
        "stitches": st.stitches,
        "jumps": st.jumps,
        "trims": st.trims,
        "colorChanges": st.color_changes,
        "blocks": blocks_json(&plan),
    }))
}

/// `found` sewn after the fills, a thread at a time: on the end of the last block when it is the
/// same thread, else in a block of its own.
fn add_features(plan: &mut StitchPlan, found: Vec<detail::Feature>, chart: &ThreadChart, travel: f64) {
    let mut threads: Vec<usize> = vec![];
    for f in &found {
        if !threads.contains(&f.thread) {
            threads.push(f.thread);
        }
    }
    let mut found: Vec<Option<detail::Feature>> = found.into_iter().map(Some).collect();
    for t in threads {
        let Some(thread) = chart.threads.get(t).map(|th| th.thread()) else { continue };
        let (mut runs, mut rings) = (vec![], vec![]);
        for f in found.iter_mut() {
            if f.as_ref().is_some_and(|f| f.thread == t)
                && let Some(f) = f.take()
            {
                runs.extend(f.runs);
                rings.extend(f.rings);
            }
        }
        let steps = join_runs(runs, &rings, travel, 2.0);
        let Some(Step::Stitch(first)) = steps.first().copied() else { continue };
        match plan.blocks.last_mut() {
            Some(b) if b.thread.rgb == thread.rgb => {
                b.steps.push(Step::Trim);
                b.steps.push(Step::Jump(first));
                b.steps.extend(steps);
            }
            Some(_) => {
                let mut all = vec![Step::Jump(first)];
                all.extend(steps);
                plan.blocks.push(Block { thread, steps: all });
            }
            None => plan.blocks.push(Block { thread, steps }),
        }
    }
}

/// A row per colour block of `plan`: its thread, stitches, jumps and trims.
pub(super) fn blocks_json(plan: &StitchPlan) -> Vec<Value> {
    plan.blocks
        .iter()
        .map(|b| {
            let count = |f: fn(&Step) -> bool| b.steps.iter().filter(|s| f(s)).count();
            json!({
                "thread": hex(b.thread.rgb),
                "name": b.thread.name,
                "stitches": count(|s| matches!(s, Step::Stitch(_))),
                "jumps": count(|s| matches!(s, Step::Jump(_))),
                "trims": count(|s| matches!(s, Step::Trim)),
            })
        })
        .collect()
}

/// The most runs of one colour put in nearest-first order (more keep the order they were made in).
const MAX_ORDERED_RUNS: usize = 6_000;

/// Every region filled along `grid`, colour by colour: one block per thread. Within a colour the
/// runs are sewn nearest first (each may be reversed), and each joins the next by travel when the
/// gap is at most `travel` mm: stitched when the straight way lies inside the colour's regions
/// (later rows cover it), else a single jump, under the machine's trim threshold; longer gaps are
/// a trim and a jump. More than `max` stitches is an error.
fn sew_regions(r: &Regions, grid: &DirectionGrid, fp: &FieldFillParams, k: f64, max: usize, travel: f64) -> Result<StitchPlan> {
    let mut blocks: Vec<Block> = vec![];
    let mut used = 0usize;
    // The runs of the current colour, and its regions' rings (mm).
    let mut runs_of: Vec<Vec<vectorcraft_stitch::Pt>> = vec![];
    let mut rings_of: Vec<Vec<(f64, f64)>> = vec![];
    let mut current: Option<vectorcraft_stitch::Thread> = None;
    let order = r.sewing_order();
    let mut flush = |thread: Option<vectorcraft_stitch::Thread>, runs: &mut Vec<Vec<vectorcraft_stitch::Pt>>, rings: &mut Vec<Vec<(f64, f64)>>| {
        if let Some(thread) = thread
            && !runs.is_empty()
        {
            let steps = join_runs(std::mem::take(runs), rings, travel, fp.max_length);
            blocks.push(Block { thread, steps });
        }
        runs.clear();
        rings.clear();
    };
    for i in order {
        let Some(tp) = r.traced.paths.get(i) else { continue };
        let Some(&t) = r.thread_of.get(i) else { continue };
        let thread = r.chart.threads.get(t).map(|th| th.thread()).ok_or_else(|| bad(TO_STITCHES, "the thread chart changed"))?;
        let rings = rings_mm(&tp.path, k, TO_STITCHES)?;
        let fallback = grid.mean_angle(&rings, FillRule::NonZero).unwrap_or(0.0);
        // Each region its own seed, from the design's, so one region's change doesn't reshuffle another.
        let seed = fp.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let params = FieldFillParams { fallback_angle: fallback, seed, ..*fp };
        let runs = field_fill(&rings, FillRule::NonZero, grid, &params, max.saturating_sub(used).saturating_add(1))
            .map_err(|e| bad(TO_STITCHES, format!("{e} (raise spacing, make the design smaller, or allow more stitches with maxStitches)")))?;
        if runs.is_empty() {
            continue;
        }
        if current.as_ref().is_none_or(|c| c.rgb != thread.rgb) {
            flush(current.take(), &mut runs_of, &mut rings_of);
            current = Some(thread);
        }
        for run in runs {
            used = used.saturating_add(run.len());
            if used > max {
                return Err(bad(
                    TO_STITCHES,
                    format!("the design has more than {max} stitches: raise spacing, make it smaller, or raise maxStitches"),
                ));
            }
            runs_of.push(run);
        }
        rings_of.extend(rings);
    }
    flush(current.take(), &mut runs_of, &mut rings_of);
    // A colour after the first starts with a jump to its first stitch.
    for b in blocks.iter_mut().skip(1) {
        if let Some(Step::Stitch(p0)) = b.steps.first().copied() {
            b.steps.insert(0, Step::Jump(p0));
        }
    }
    if blocks.is_empty() {
        return Err(bad(TO_STITCHES, "the photo's regions are too small to fill: lower minArea, or make the design larger"));
    }
    let plan = StitchPlan { name: "photo".into(), blocks };
    plan.check().map_err(|e| bad(TO_STITCHES, e))?;
    Ok(plan)
}

/// `runs` of one colour as steps: nearest first (see [`sew_regions`]), joined by travel inside
/// `rings` (mm, non-zero), by a jump, or by a trim and a jump.
pub(super) fn join_runs(mut runs: Vec<Vec<vectorcraft_stitch::Pt>>, rings: &[Vec<(f64, f64)>], travel: f64, stitch: f64) -> Vec<Step> {
    if runs.len() <= MAX_ORDERED_RUNS {
        runs = nearest_first(runs);
    }
    let mut steps: Vec<Step> = vec![];
    let mut last: Option<vectorcraft_stitch::Pt> = None;
    for run in runs {
        let Some(&start) = run.first() else { continue };
        if let Some(from) = last {
            let gap = from.dist(start) / 10.0;
            if gap <= travel {
                if inside_all_the_way(from, start, rings) {
                    // Stitched travel, in stitches no longer than `stitch`.
                    let n = (gap / stitch.max(0.5)).ceil().clamp(1.0, 1_000.0) as i32;
                    for s in 1..n {
                        let t = f64::from(s) / f64::from(n);
                        let (a, b) = (from.to_mm(), start.to_mm());
                        if let Some(q) = vectorcraft_stitch::Pt::from_mm(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t) {
                            steps.push(Step::Stitch(q));
                        }
                    }
                } else {
                    steps.push(Step::Jump(start));
                }
            } else {
                steps.push(Step::Trim);
                steps.push(Step::Jump(start));
            }
        }
        last = run.last().copied();
        steps.extend(run.into_iter().map(Step::Stitch));
    }
    steps
}

/// `runs` reordered: the first stays first, then each time the run whose end is nearest to where the
/// last one ended, reversed when its far end is the nearer.
fn nearest_first(runs: Vec<Vec<vectorcraft_stitch::Pt>>) -> Vec<Vec<vectorcraft_stitch::Pt>> {
    let mut left: Vec<Option<Vec<vectorcraft_stitch::Pt>>> = runs.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(left.len());
    let mut at: Option<vectorcraft_stitch::Pt> = None;
    loop {
        let mut best: Option<(f64, usize, bool)> = None;
        for (i, r) in left.iter().enumerate() {
            let Some(r) = r else { continue };
            let (Some(&a), Some(&b)) = (r.first(), r.last()) else { continue };
            let Some(p) = at else {
                best = Some((0.0, i, false));
                break;
            };
            for (d, rev) in [(p.dist(a), false), (p.dist(b), true)] {
                if best.is_none_or(|x| d < x.0) {
                    best = Some((d, i, rev));
                }
            }
        }
        let Some((_, i, rev)) = best else { break };
        let Some(mut r) = left.get_mut(i).and_then(Option::take) else { break };
        if rev {
            r.reverse();
        }
        at = r.last().copied();
        out.push(r);
    }
    // Runs without points are dropped above; none have any stitches to sew.
    out
}

/// Whether the straight way from `a` to `b` stays inside `rings` (mm, non-zero), checked every
/// quarter of a millimetre.
fn inside_all_the_way(a: vectorcraft_stitch::Pt, b: vectorcraft_stitch::Pt, rings: &[Vec<(f64, f64)>]) -> bool {
    let (a, b) = (a.to_mm(), b.to_mm());
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / 0.25).ceil().clamp(1.0, 400.0) as usize;
    (0..=n).all(|s| {
        let t = s as f64 / n as f64;
        winding(rings, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)) != 0
    })
}

/// The winding number of `rings` around `p`.
pub(super) fn winding(rings: &[Vec<(f64, f64)>], p: (f64, f64)) -> i32 {
    let mut w = 0;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { continue };
            let cross = (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1);
            if a.1 <= p.1 {
                if b.1 > p.1 && cross > 0.0 {
                    w += 1;
                }
            } else if b.1 <= p.1 && cross < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

fn synthetic_cmd(_: &mut Session, p: &Value) -> Result<Value> {
    let w = ranged(p, "width", 480.0, (16.0, 2048.0), SYNTHETIC)?.round() as u32;
    let h = ranged(p, "height", 360.0, (16.0, 2048.0), SYNTHETIC)?.round() as u32;
    let seed = ranged(p, "seed", 1.0, (0.0, f64::from(u32::MAX)), SYNTHETIC)?.round() as u32;
    let out_path = text_param(p, "path", SYNTHETIC)?;
    let png = encode_png(&synthetic_fur(w, h, seed)).map_err(|e| bad(SYNTHETIC, e))?;
    let ((ex, ey), er) = synthetic_fur_eye(w, h);
    let mut v = json!({
        "width": w,
        "height": h,
        "angles": [FUR_ANGLES.0, FUR_ANGLES.1],
        "eye": {"x": round2(ex), "y": round2(ey), "r": round2(er)},
    });
    match out_path {
        Some(o) => {
            super::super::fileio::write_file(o, &png)?;
            v["path"] = json!(o);
        }
        None => v["pngBase64"] = json!(vectorcraft_format::base64_encode(&png)),
    }
    Ok(v)
}

#[cfg(test)]
#[path = "stitches_tests.rs"]
mod tests;
