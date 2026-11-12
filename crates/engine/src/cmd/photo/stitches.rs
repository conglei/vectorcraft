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
use vectorcraft_trace::{DirectionField, FieldParams, Method, Mode, Palette, TraceParams, TraceResult, direction_field, trace_within};

use super::super::fileio::stitchfile;
use super::super::stitchthreads;
use super::super::{CommandSpec, Result, Session, always, bad, cmd, number_or, text_param};
use super::{MAX_PHOTO_BYTES, chosen_provider, read_photo};

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
            "{file?: | image?: (the photo, as photo.palette), …the other params of photo.palette (colors, chart, maxSide, blur, minArea, widthMm, heightMm), detail?: 40 (0–100: how closely outlines follow the photo; lower is smoother with fewer points), x?: 0, y?: 0 (the top-left corner, document units), angles?: true (each region's tatami angle is its mean direction from photo.directionField; false: 0), cell?: 2, smoothing?: 2 (mm: as photo.directionField)} places the photo's colour regions in the active document (a new one when none is open) as one group of filled paths, a group per thread colour, each filled with its thread's colour and set to sew as tatami (stitch.setType) at its region's direction; the paths are ordinary art to edit; one undo step → {id (the group), colors (as photo.palette), regions, document (index of the document)}",
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
            "{file?: | image?: (the photo, as photo.palette), provider?: none (use the photo as it is) | nomodel | gemini (as photo.prepare; not given: the photoProvider preference, default nomodel; gemini sends the photo to Google's Gemini API, which cuts the subject out), colors?: 8 (7–16), chart?, maxSide?: 600, blur?: 1.5, minArea?: 4, detail?: 40, widthMm?: 100, heightMm?, fill?: field (default: rows that follow the photo's direction field) | tatami (straight rows, each region at its mean direction, left as editable regions), spacing?: 0.4 (mm between rows, 0.2–5), minLength?: 2, maxLength?: 4 (mm, 0.5–12: each stitch's length is drawn at random between them), feather?: 0 (mm, 0–10: row ends pulled in by a random amount up to this, for a ragged fur edge), featherEdges?: both (default) | start | end (the ends going along the region's direction), seed?: 1 (the same seed sews the same), cell?: 2, smoothing?: 2 (mm, as photo.directionField), maxStitches?: 100000 (at most 1000000: more is an error)} turns a photo into a stitch design in a new document: prepare (unless none), thread colours (photo.palette), regions (photo.regions), direction field, fills, sewn colour by colour, the colour covering most first. With fill: field the document holds the stitches (needle points, exported unchanged, lock stitches included) over a template layer of the regions; with tatami it holds the regions, sewn by their stitch properties. document.export {format: dst} writes it → {document (index), fill, provider, widthMm, heightMm, colors (as photo.palette), regions, stitches, jumps, trims, colorChanges, warnings}",
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
}

/// The photo's colour regions: softened by `blur`, reduced to `colors`, small patches merged, as
/// outlines in pixels; each colour matched to the chart.
fn regions(s: &Session, p: &Value, img: &Raster, k: f64, cmd: &str) -> Result<Regions> {
    let colors = ranged(p, "colors", 8.0, COLORS, cmd)?.round() as u32;
    let blur = ranged(p, "blur", 1.5, (0.0, 8.0), cmd)? as f32;
    let min_area = ranged(p, "minArea", 4.0, (0.0, 10_000.0), cmd)?;
    let detail = ranged(p, "detail", 40.0, (0.0, 100.0), cmd)?;
    let (chart_name, chart) = chart_for(s, p, cmd)?;
    let soft = if blur > 0.0 {
        let rgba =
            image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone()).ok_or_else(|| bad(cmd, "the image's pixels don't fit its size"))?;
        let b = image::imageops::blur(&rgba, blur);
        Raster { width: img.width, height: img.height, rgba: b.into_raw() }
    } else {
        img.clone()
    };
    let noise = (min_area / (k * k)).round().clamp(1.0, 1e6) as u32;
    let params = TraceParams {
        mode: Mode::Color,
        palette: Palette::Limited,
        colors,
        paths: detail,
        corners: 50.0,
        noise,
        method: Method::Abutting,
        fills: true,
        strokes: false,
        ..TraceParams::default()
    };
    let traced = trace_within(&soft, &params, MAX_REGION_ANCHORS).map_err(|e| bad(cmd, format!("{e}: raise minArea or lower detail or colors")))?;
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
    let mut by_thread: Vec<ThreadUse> = vec![];
    for tp in &traced.paths {
        let m = chart.nearest(tp.color).ok_or_else(|| bad(cmd, "the thread chart is empty"))?;
        thread_of.push(m.index);
        let entry = match by_thread.iter_mut().position(|e| e.0 == m.index) {
            Some(i) => by_thread.get_mut(i),
            None => {
                by_thread.push((m.index, 0, 0, vec![]));
                by_thread.last_mut()
            }
        };
        if let Some(e) = entry {
            e.1 += tp.pixels;
            e.2 += 1;
            match e.3.iter_mut().find(|c| c.0 == tp.color) {
                Some(c) => c.1 += tp.pixels,
                None => e.3.push((tp.color, tp.pixels)),
            }
        }
    }
    by_thread.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let colors = by_thread
        .into_iter()
        .map(|(t, px, n, cs)| {
            let main = cs.iter().max_by_key(|c| c.1).map(|c| c.0).unwrap_or([0, 0, 0]);
            let de = chart.threads.get(t).map_or(0.0, |th| vectorcraft_stitch::delta_e(main, th.rgb));
            (t, px, n, main, de)
        })
        .collect();
    Ok(Regions { traced, thread_of, colors, chart, chart_name })
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

/// The fill settings of photo.toStitches.
fn fill_params(p: &Value) -> Result<FieldFillParams> {
    let c = TO_STITCHES;
    let spacing = ranged(p, "spacing", 0.4, (0.2, 5.0), c)?;
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

fn to_stitches_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let c = TO_STITCHES;
    let tatami = match text_param(p, "fill", c)? {
        None | Some("field") => false,
        Some("tatami") => true,
        Some(o) => return Err(bad(c, format!("fill `{o}`: field or tatami"))),
    };
    let fp = fill_params(p)?;
    let max_stitches = ranged(p, "maxStitches", DEFAULT_MAX_STITCHES, (1.0, MAX_MAX_STITCHES), c)?.round() as usize;
    let (img, provider, mut warnings) = worked(s, p, c, true)?;
    let k = mm_per_px(p, img.width, img.height, c)?;
    let r = regions(s, p, &img, k, c)?;
    let grid = grid_mm(&field(p, &img, k, c)?, k);
    let (w_mm, h_mm) = (f64::from(img.width) * k, f64::from(img.height) * k);
    let mut doc = if tatami {
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
        let plan = sew_regions(&r, &grid, &fp, k, max_stitches)?;
        let machine = profile("commercial15").ok_or_else(|| bad(c, "the machine profile is missing"))?;
        let prepared = prepare(&plan, &machine, &vectorcraft_stitch::PrepareOptions { locks: true, travel_trims: true }).map_err(|e| bad(c, e))?;
        warnings.extend(prepared.warnings);
        let mut d = to_document(&prepared.plan);
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
    doc.units = Unit::Millimeters;
    doc.title = "Photo stitches".into();
    let margin = 10.0 * PT_PER_MM;
    let board = Rect::new(0.0, 0.0, w_mm * PT_PER_MM, h_mm * PT_PER_MM).inflate(margin, margin);
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
        "fill": if tatami { "tatami" } else { "field" },
        "provider": provider,
        "widthMm": round2(w_mm),
        "heightMm": round2(h_mm),
        "chart": r.chart_name,
        "colors": r.colors_json(),
        "regions": r.traced.paths.len(),
        "stitches": st.stitches,
        "jumps": st.jumps,
        "trims": st.trims,
        "colorChanges": st.color_changes,
        "warnings": warnings,
    }))
}

/// Every region filled along `grid`, colour by colour: one block per thread, runs joined by a trim
/// and a jump. More than `max` stitches is an error.
fn sew_regions(r: &Regions, grid: &DirectionGrid, fp: &FieldFillParams, k: f64, max: usize) -> Result<StitchPlan> {
    let mut blocks: Vec<Block> = vec![];
    let mut used = 0usize;
    for i in r.sewing_order() {
        let Some(tp) = r.traced.paths.get(i) else { continue };
        let Some(&t) = r.thread_of.get(i) else { continue };
        let thread = r.chart.threads.get(t).map(|th| th.thread()).ok_or_else(|| bad(TO_STITCHES, "the thread chart changed"))?;
        let rings = rings_mm(&tp.path, k, TO_STITCHES)?;
        let fallback = grid.mean_angle(&rings, FillRule::NonZero).unwrap_or(0.0);
        // Each region its own seed, from the design's, so one region's change doesn't reshuffle another.
        let seed = fp.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let params = FieldFillParams { fallback_angle: fallback, seed, ..*fp };
        let runs = field_fill(&rings, FillRule::NonZero, grid, &params, max.saturating_sub(used).saturating_add(1))
            .map_err(|e| bad(TO_STITCHES, format!("{e} (raise spacing, make the design smaller, or raise maxStitches)")))?;
        if runs.is_empty() {
            continue;
        }
        let new_block = blocks.last().is_none_or(|b| b.thread.rgb != thread.rgb);
        if new_block {
            blocks.push(Block { thread, steps: vec![] });
        }
        let Some(block) = blocks.last_mut() else { continue };
        for run in runs {
            let Some(&start) = run.first() else { continue };
            if !block.steps.is_empty() {
                block.steps.push(Step::Trim);
                block.steps.push(Step::Jump(start));
            } else if !new_block {
                block.steps.push(Step::Jump(start));
            }
            used = used.saturating_add(run.len());
            if used > max {
                return Err(bad(
                    TO_STITCHES,
                    format!("the design has more than {max} stitches: raise spacing, make it smaller, or raise maxStitches"),
                ));
            }
            block.steps.extend(run.into_iter().map(Step::Stitch));
        }
    }
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
