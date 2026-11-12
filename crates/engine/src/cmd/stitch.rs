//! Embroidery queries: `stitch.info`, the statistics and warnings a DST, DSB or EXP export with the
//! same options would give, without writing a file, and `stitch.preview`, a picture of the stitches
//! that export would sew. The stitch properties of objects are in [`objects`].

mod objects;
mod sequence;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::Affine;
use vectorcraft_stitch::{
    MAX_SCALE, MAX_THREAD_MM, MIN_SCALE, MIN_THREAD_MM, PT_PER_MM, PreviewMode, PreviewOptions, StitchPlan, THREAD_WIDTH_MM, calibration, preview,
    preview_size, stats,
};

use super::edit::roots_of;
use super::fileio::stitchfile;
use super::stitchthreads;
use super::{CommandSpec, Result, Session, bad, cmd, has_doc, number_or, switch_or, text_param};

const C: &str = "stitch.info";
const CALIBRATION: &str = "stitch.calibration";
const PREVIEW: &str = "stitch.preview";
/// The largest preview returned inline as base64 (2048 × 2048 pixels); larger ones need `path`.
const MAX_INLINE_PIXELS: u64 = 2048 * 2048;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![
        cmd!(
            query "stitch.info",
            "Stitch Info",
            [],
            None,
            "{format?: dst (default) | dsb | exp, machine?: commercial15 (default; 15 needles, a 500 x 350 mm field) | bai_mirror (a commercial single-head 15-needle machine, 350 x 500 mm; the same limits, and a warning for a format its panel does not read: DST and DSB) | generic (no needle or field limits), mode?: run (running stitch along each stroked path) | manual (one needle point per anchor) (given: for all art; not given: art on the layer an opened stitch file made is manual, so its needle points export unchanged, and all other art is run), stitchLength?: 2.5 (mm, 0.5–12), minStitch?: 0.3 (mm, 0–2: shorter stitches are dropped), maxStitch?: 12.1 (mm, 1–12.1; EXP and DSB 12.7, their default too: longer stitches are split into equal pieces, jumps and a final stitch), locks?: (lock stitches at the start and end of each run; not given: off when everything stitched is on an opened stitch file's layer, on otherwise, with a warning when that file's runs get them too), travelTrims?: (a trim before travel long enough that the machine would read it as one; true: for all art, false: for none, the trims the plan has stay either way; not given: off when everything stitched is on an opened stitch file's layer, which keeps its own trims, on otherwise), jumpsPerTrim?: 3 (2–8: jumps in a row the machine reads as a trim), trimBeforeColorChange?: false, start?: center (default) | firstStitch (the design start, stitch.designStartEnd.set, replaces it when set), returnToStart?: true (the file ends with travel back to its origin; not given: off when everything stitched is on an opened stitch file's layer, on otherwise), name?: (the design name; default the document's title), selectedOnly?: false (the selected objects alone), fill?: {type?: tatami (default; rows of running stitch, holes kept by the fill rule, sewn before the outline) | none, angle?: 0 (degrees, counter-clockwise, -360–360), spacing?: 0.4 (mm between rows, 0.2–5), stitchLength?: 3 (mm, 1–8), stagger?: 0.25 (row offset as a fraction of the stitch length, 0–1)}, stroke?: {type?: run (default) | satin (a satin column centred on the path, square ends, stitches over maxStitch split), spacing?: 0.4 (satin, mm peak to peak, 0.2–5), width?: (satin, mm, 0.5–20; default the stroke weight, held to 0.5–20)} (fill and satin are not used on the layer an opened stitch file made)} what document.export {format: dst|dsb|exp} with the same options would write → {stitches, jumps, trims, colorChanges, records (the records in the file, for DST its header's stitch count: travel, trims and the end record included), widthMm, heightMm, travelMm (the distance jumped between runs, to 0.1 mm; the writer's return to the origin aside), threads: [{rgb: \"#rrggbb\", stitches, lengthMm, brand?, code?, name?, deltaE? (with a thread chart active, stitch.threads.use: the chart's nearest thread to this colour and how far off it is, CIE76)}] (one per thread colour, in sewing order), warnings}; a document with no stroked or filled paths is an error",
            has_doc,
            info
        ),
        cmd!(
            "stitch.calibration",
            "Stitch Calibration Design",
            ["Embroidery"],
            None,
            "{x?: 0, y?: 0 (the design's top-left corner, document units)} adds a 100 mm test piece for a first stitch-out, as one group of paths stroked in three thread colours: a square, scale ticks along its top edge, an F that shows direction and mirroring, and a near and a far pair of squares, each pair trimmed apart, that show whether trims happen where the file asks; it stitches off-centre from the square's top-left corner; one undo step → {id} of the group",
            has_doc,
            add_calibration
        ),
        cmd!(
            query "stitch.preview",
            "Stitch Preview",
            [],
            None,
            "{…the plan options of stitch.info (format, machine, mode, stitchLength, minStitch, maxStitch, locks, jumpsPerTrim, trimBeforeColorChange, start, returnToStart, name, selectedOnly: the preview shows the stitches that export would write), view?: realistic (default: each stitch a lit thread from needle point to needle point, later stitches over earlier ones) | density (1 mm cells coloured by thread coverage, stitched length × thread width / cell area: pale green little, green 1 (one layer), yellow 1.5, red 2 and above: too dense), scale?: 10 (pixels per mm, 0.5–50), threadWidth?: 0.4 (mm, 0.1–2), fabric?: \"#ece8de\" (the background), showJumps?: false (jumps as dashed lines), showNeedles?: false (a dot at each needle point), showFunctions?: false (rings: red at trims, the new thread's colour at colour changes, orange at stops), path?: (write the PNG there)} a PNG of the stitches with a 2 mm margin, at most 8192 pixels a side → {width, height, path, warnings (the plan's: stitch.info's less those of the file writer)}; without path {width, height, pngBase64, warnings} (at most 2048 × 2048 pixels: pass path or a lower scale for more); the density view adds {maxCoverage, denseCells}; a document with no stroked paths is an error",
            has_doc,
            preview_cmd
        ),
    ];
    v.extend(objects::specs());
    v.extend(sequence::specs());
    v
}

fn add_calibration(s: &mut Session, p: &Value) -> Result<Value> {
    let x = number_or(p, "x", 0.0, CALIBRATION)?;
    let y = number_or(p, "y", 0.0, CALIBRATION)?;
    if !(x.is_finite() && y.is_finite()) {
        return Err(bad(CALIBRATION, "x and y must be finite"));
    }
    let parent = s.doc()?.target_parent()?;
    let shift = Affine::translate((x, y));
    let id = s.edit("Stitch Calibration Design", |d, sel| {
        let mut paths: Vec<Arc<Node>> = Vec::new();
        for part in calibration() {
            let [r, g, b] = part.rgb;
            let look = Appearance::basic(Paint::None, Paint::solid(Color::rgb8(r, g, b)), THREAD_WIDTH_MM * PT_PER_MM);
            let mut n = Node::path(d.alloc_id(), part.path.transformed(shift), look);
            n.name = Some(part.name.to_string());
            paths.push(Arc::new(n));
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, paths);
        g.name = Some("Stitch Calibration".into());
        d.insert(parent, usize::MAX, g)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    let format = text_param(p, "format", C)?.unwrap_or("dst");
    let selection = selected(s, p, C)?;
    let (plan, out) = stitchfile::written(C, &s.doc()?.doc, p, format, selection)?;
    let st = stats(&plan);
    // With a thread chart active, each colour names the nearest thread of it.
    let chart = stitchthreads::active(s);
    let threads: Vec<Value> = st
        .threads
        .iter()
        .map(|t| {
            let [r, g, b] = t.rgb;
            let mut v = json!({"rgb": format!("#{r:02x}{g:02x}{b:02x}"), "stitches": t.stitches, "lengthMm": t.length_mm});
            if let Some((_, c)) = &chart
                && let Some(m) = c.nearest(t.rgb)
                && let Some(thread) = c.threads.get(m.index)
            {
                v["brand"] = json!(thread.brand);
                v["code"] = json!(thread.code);
                v["name"] = json!(thread.name);
                v["deltaE"] = json!((m.delta_e * 100.0).round() / 100.0);
            }
            v
        })
        .collect();
    Ok(json!({
        "stitches": st.stitches,
        "jumps": st.jumps,
        "trims": st.trims,
        "colorChanges": st.color_changes,
        "records": out.records,
        "widthMm": st.width_mm,
        "heightMm": st.height_mm,
        "travelMm": (st.travel_mm * 10.0).round() / 10.0,
        "threads": threads,
        "warnings": out.warnings,
    }))
}

/// The objects `selectedOnly` names, if it is on.
pub(super) fn selected(s: &Session, p: &Value, cmd: &str) -> Result<Option<Vec<NodeId>>> {
    if !switch_or(p, "selectedOnly", false, cmd)? {
        return Ok(None);
    }
    let st = s.doc()?;
    let roots = roots_of(&st.doc, st.selection.in_paint_order(&st.doc));
    if roots.is_empty() {
        return Err(bad(cmd, "select something to stitch (or turn off selectedOnly)"));
    }
    Ok(Some(roots))
}

/// A `#rrggbb` colour.
fn hex_rgb(v: &str) -> Option<[u8; 3]> {
    let h = v.strip_prefix('#').filter(|h| h.len() == 6 && h.is_ascii())?;
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// The number `key` in `p` (default `default`), finite and within `lo..=hi`.
fn ranged(p: &Value, key: &str, default: f64, (lo, hi): (f64, f64), unit: &str) -> Result<f64> {
    let v = number_or(p, key, default, PREVIEW)?;
    if !(v.is_finite() && (lo..=hi).contains(&v)) {
        return Err(bad(PREVIEW, format!("{key} must be {lo}–{hi} {unit}, not {v}")));
    }
    Ok(v)
}

/// The renderer options in `p`.
fn preview_options(p: &Value) -> Result<PreviewOptions> {
    let d = PreviewOptions::default();
    let mode = match text_param(p, "view", PREVIEW)?.unwrap_or("realistic") {
        "realistic" => PreviewMode::Realistic,
        "density" => PreviewMode::Density,
        other => return Err(bad(PREVIEW, format!("view `{other}`: realistic or density"))),
    };
    let fabric = match text_param(p, "fabric", PREVIEW)? {
        None => d.fabric,
        Some(v) => hex_rgb(v).ok_or_else(|| bad(PREVIEW, format!("fabric `{v}`: a colour as #rrggbb")))?,
    };
    Ok(PreviewOptions {
        scale: ranged(p, "scale", d.scale, (MIN_SCALE, MAX_SCALE), "pixels per mm")?,
        thread_mm: ranged(p, "threadWidth", d.thread_mm, (MIN_THREAD_MM, MAX_THREAD_MM), "mm")?,
        fabric,
        show_jumps: switch_or(p, "showJumps", false, PREVIEW)?,
        show_needles: switch_or(p, "showNeedles", false, PREVIEW)?,
        show_functions: switch_or(p, "showFunctions", false, PREVIEW)?,
        mode,
    })
}

/// `plan` drawn with `o` as PNG bytes, with its width, height and, in the density view, figures.
fn preview_png(plan: &StitchPlan, o: &PreviewOptions) -> Result<(Vec<u8>, Value)> {
    let img = preview(plan, o).map_err(|e| bad(PREVIEW, e))?;
    let (w, h) = (img.width, img.height);
    let mut info = json!({"width": w, "height": h});
    if o.mode == PreviewMode::Density {
        info["maxCoverage"] = json!((img.max_coverage * 1000.0).round() / 1000.0);
        info["denseCells"] = json!(img.dense_cells);
    }
    let rgba = image::RgbaImage::from_raw(w, h, img.rgba).ok_or_else(|| bad(PREVIEW, "the preview's pixels don't fit its size"))?;
    let mut png = Vec::new();
    rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| bad(PREVIEW, e.to_string()))?;
    Ok((png, info))
}

fn preview_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let format = text_param(p, "format", PREVIEW)?.unwrap_or("dst");
    let o = preview_options(p)?;
    let path = text_param(p, "path", PREVIEW)?;
    let selection = selected(s, p, PREVIEW)?;
    let (plan, warnings) = stitchfile::plan(PREVIEW, &s.doc()?.doc, p, format, selection)?;
    let (w, h) = preview_size(&plan, &o).map_err(|e| bad(PREVIEW, e))?;
    if path.is_none() && u64::from(w) * u64::from(h) > MAX_INLINE_PIXELS {
        return Err(bad(PREVIEW, format!("a {w} × {h} preview is too large to return inline: pass path, or a lower scale")));
    }
    let (png, mut out) = preview_png(&plan, &o)?;
    match path {
        Some(path) => {
            super::fileio::write_file(path, &png)?;
            out["path"] = json!(path);
        }
        None => out["pngBase64"] = json!(vectorcraft_format::base64_encode(&png)),
    }
    out["warnings"] = json!(warnings);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::Session;

    /// The golden preview of the calibration design (`stitch.preview {scale: 2, showFunctions: true}`).
    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stitch/calibration-preview.png");

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        s
    }

    fn png(r: &Value) -> image::RgbaImage {
        let bytes = vectorcraft_format::base64_decode(r["pngBase64"].as_str().expect("pngBase64")).unwrap();
        image::load_from_memory(&bytes).unwrap().to_rgba8()
    }

    #[test]
    fn one_stroked_path_previews_at_the_size_of_its_stitches() {
        let mut s = session();
        s.execute("paint.setFill", &json!({"none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        let info = s.execute("stitch.info", &json!({})).unwrap();
        let r = s.execute("stitch.preview", &json!({})).unwrap();
        let side = |mm: &Value| ((mm.as_f64().unwrap() + 4.0) * 10.0).round() as u32;
        assert_eq!(r["width"], side(&info["widthMm"]), "{info} {r}");
        assert_eq!(r["height"], side(&info["heightMm"]), "{info} {r}");
        let img = png(&r);
        assert_eq!((img.width(), img.height()), (r["width"].as_u64().unwrap() as u32, r["height"].as_u64().unwrap() as u32));
        // Red thread along the top edge, 2 mm in from the corner; fabric in the middle.
        let top = img.get_pixel(img.width() / 2, 20).0;
        assert!(top[0] > top[1] + 60, "{top:?}");
        assert_eq!(img.get_pixel(img.width() / 2, img.height() / 2).0, [236, 232, 222, 255]);
        // At 5 px per mm it is half as large, give or take a pixel.
        let half = s.execute("stitch.preview", &json!({"scale": 5})).unwrap();
        assert!(half["width"].as_i64().unwrap().abs_diff(r["width"].as_i64().unwrap() / 2) <= 1, "{half}");
    }

    #[test]
    fn the_calibration_design_matches_its_golden_preview() {
        let mut s = session();
        s.execute("stitch.calibration", &json!({})).unwrap();
        let r = s.execute("stitch.preview", &json!({"scale": 2, "showFunctions": true})).unwrap();
        let img = png(&r);
        if std::env::var_os("VECTORCRAFT_BLESS").is_some() {
            std::fs::create_dir_all(std::path::Path::new(GOLDEN).parent().unwrap()).unwrap();
            img.save(GOLDEN).unwrap();
        }
        let golden = image::open(GOLDEN).expect("the golden preview (VECTORCRAFT_BLESS=1 writes it)").to_rgba8();
        assert_eq!(img.dimensions(), golden.dimensions());
        let off = img.pixels().zip(golden.pixels()).filter(|(a, b)| a.0.iter().zip(b.0).any(|(x, y)| x.abs_diff(y) > 6)).count();
        assert_eq!(off, 0, "{off} pixels differ from {GOLDEN} by more than 6");
    }

    #[test]
    fn the_density_view_reports_coverage() {
        let mut s = session();
        s.execute("stitch.calibration", &json!({})).unwrap();
        let r = s.execute("stitch.preview", &json!({"view": "density", "scale": 2})).unwrap();
        assert!(r["maxCoverage"].as_f64().unwrap() > 0.0, "{r}");
        assert!(r["denseCells"].is_u64(), "{r}");
        assert!(r.get("pngBase64").is_some());
        let realistic = s.execute("stitch.preview", &json!({"scale": 2})).unwrap();
        assert!(realistic.get("maxCoverage").is_none());
        assert_ne!(png(&realistic), png(&r));
    }

    #[test]
    fn a_preview_writes_to_a_path() {
        let mut s = session();
        s.execute("stitch.calibration", &json!({})).unwrap();
        let path = std::env::temp_dir().join(format!("vc-stitch-preview-{}.png", std::process::id()));
        let r = s.execute("stitch.preview", &json!({"scale": 1, "path": path.to_string_lossy()})).unwrap();
        assert!(r.get("pngBase64").is_none(), "{r}");
        let img = image::open(&path).unwrap();
        assert_eq!((img.width(), img.height()), (104, 104));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bad_options_and_large_inline_previews_are_refused() {
        let mut s = session();
        s.execute("stitch.calibration", &json!({})).unwrap();
        for p in [
            json!({"scale": 0}),
            json!({"scale": 51}),
            json!({"threadWidth": 3}),
            json!({"view": "3d"}),
            json!({"fabric": "white"}),
            json!({"fabric": "#ééé"}),
            json!({"format": "pes"}),
        ] {
            assert!(s.execute("stitch.preview", &p).is_err(), "{p}");
        }
        // 104 mm at 25 px per mm is 2600 pixels a side: too large inline.
        let e = s.execute("stitch.preview", &json!({"scale": 25})).unwrap_err().to_string();
        assert!(e.contains("pass path"), "{e}");
        // Nothing stroked: an error.
        assert!(session().execute("stitch.preview", &json!({})).is_err());
    }
}
