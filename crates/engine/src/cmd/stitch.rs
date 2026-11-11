//! Embroidery queries: `stitch.info`, the statistics and warnings a DST or EXP export with the
//! same options would give, without writing a file.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::Affine;
use vectorcraft_stitch::{PT_PER_MM, THREAD_WIDTH_MM, calibration, stats};

use super::edit::roots_of;
use super::fileio::stitchfile;
use super::{CommandSpec, Result, Session, bad, cmd, has_doc, number_or, switch_or, text_param};

const C: &str = "stitch.info";
const CALIBRATION: &str = "stitch.calibration";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stitch.info",
            "Stitch Info",
            [],
            None,
            "{format?: dst (default) | exp, machine?: commercial15 (default; 15 needles, a 500 x 350 mm field) | generic (no needle or field limits), mode?: run (running stitch along each stroked path) | manual (one needle point per anchor) (given: for all art; not given: art on the layer an opened stitch file made is manual, so its needle points export unchanged, and all other art is run), stitchLength?: 2.5 (mm, 0.5–12), minStitch?: 0.3 (mm, 0–2: shorter stitches are dropped), maxStitch?: 12.1 (mm, 1–12.1; EXP 12.7, its default too: longer stitches are split by jumps), locks?: (lock stitches at the start and end of each run; not given: off when everything stitched is on an opened stitch file's layer, on otherwise, with a warning when that file's runs get them too), jumpsPerTrim?: 3 (2–8: jumps in a row the machine reads as a trim), trimBeforeColorChange?: false, start?: center (default) | firstStitch, returnToStart?: true, name?: (the design name; default the document's title), selectedOnly?: false (the selected objects alone)} what document.export {format: dst|exp} with the same options would write → {stitches, jumps, trims, colorChanges, records (the records in the file, for DST its header's stitch count: travel, trims and the end record included), widthMm, heightMm, threads: [{rgb: \"#rrggbb\", stitches, lengthMm}] (one per thread colour, in sewing order), warnings}; a document with no stroked paths is an error",
            has_doc,
            info
        ),
        cmd!(
            "stitch.calibration",
            "Stitch Calibration Design",
            [],
            None,
            "{x?: 0, y?: 0 (the design's top-left corner, document units)} adds a 100 mm test piece for a first stitch-out, as one group of paths stroked in three thread colours: a square, scale ticks along its top edge, an F that shows direction and mirroring, and a near and a far pair of squares, each pair trimmed apart, that show whether trims happen where the file asks; it stitches off-centre from the square's top-left corner; one undo step → {id} of the group",
            has_doc,
            add_calibration
        ),
    ]
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
    let st = s.doc()?;
    let selection = if switch_or(p, "selectedOnly", false, C)? {
        let roots = roots_of(&st.doc, st.selection.in_paint_order(&st.doc));
        if roots.is_empty() {
            return Err(bad(C, "select something to stitch (or turn off selectedOnly)"));
        }
        Some(roots)
    } else {
        None
    };
    let (plan, out) = stitchfile::written(C, &st.doc, p, format, selection)?;
    let st = stats(&plan);
    let threads: Vec<Value> = st
        .threads
        .iter()
        .map(|t| {
            let [r, g, b] = t.rgb;
            json!({"rgb": format!("#{r:02x}{g:02x}{b:02x}"), "stitches": t.stitches, "lengthMm": t.length_mm})
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
        "threads": threads,
        "warnings": out.warnings,
    }))
}
