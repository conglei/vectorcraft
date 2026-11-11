//! Embroidery queries: `stitch.info`, the statistics and warnings a DST or EXP export with the
//! same options would give, without writing a file.

use serde_json::{Value, json};
use vectorcraft_stitch::stats;

use super::edit::roots_of;
use super::fileio::stitchfile;
use super::{CommandSpec, Result, Session, bad, cmd, has_doc, switch_or, text_param};

const C: &str = "stitch.info";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "stitch.info",
        "Stitch Info",
        [],
        None,
        "{format?: dst (default) | exp, machine?: commercial15 (default; 15 needles, a 500 x 350 mm field) | generic (no needle or field limits), mode?: run (default; running stitch along each stroked path) | manual (one needle point per anchor), stitchLength?: 2.5 (mm, 0.5–12), minStitch?: 0.3 (mm, 0–2: shorter stitches are dropped), maxStitch?: 12.1 (mm, 1–12.1; EXP 12.7, its default too: longer stitches are split by jumps), locks?: true (lock stitches at the start and end of each run), jumpsPerTrim?: 3 (2–8: jumps in a row the machine reads as a trim), trimBeforeColorChange?: false, start?: center (default) | firstStitch, returnToStart?: true, name?: (the design name; default the document's title), selectedOnly?: false (the selected objects alone)} what document.export {format: dst|exp} with the same options would write → {stitches, jumps, trims, colorChanges, widthMm, heightMm, threads: [{rgb: \"#rrggbb\", stitches, lengthMm}] (one per thread colour, in sewing order), warnings}; a document with no stroked paths is an error",
        has_doc,
        info
    )]
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
    let (plan, warnings) = stitchfile::plan(C, &st.doc, p, format, selection)?;
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
        "widthMm": st.width_mm,
        "heightMm": st.height_mm,
        "threads": threads,
        "warnings": warnings,
    }))
}
