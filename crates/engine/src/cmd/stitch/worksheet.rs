//! Embroidery › Production Worksheet… and Export for Production…: a one-page PDF for the machine
//! operator (`stitch.worksheet`) and a folder holding the machine files and the worksheet
//! (`stitch.exportProduction`).
//!
//! The worksheet is built as a document (type, rectangles and the stitch preview as an image) and
//! written by the PDF export, so the renderer and the PDF writer do all the drawing. Its layout is
//! original: the design picture and the figures side by side, the stop table below.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, ImageBlob, ImageObject, Node, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_stitch::{Finding, Level, MAX_SIDE, PreviewMode, PreviewOptions, Step, StitchPlan, UNITS_PER_MM, assign_needles, preview, stats};

use super::super::fileio::{self, stitchfile};
use super::super::{CommandSpec, Result, Session, bad, cmd, has_doc, switch_or, text_param};
use super::production::{self, duration, estimates, findings};

const WORKSHEET: &str = "stitch.worksheet";
const EXPORT: &str = "stitch.exportProduction";

/// A4 portrait, in points.
const PAGE: (f64, f64) = (595.276, 841.89);
const MARGIN: f64 = 36.0;
/// The longest side of the design picture, in pixels.
const PICTURE_PX: f64 = 1000.0;
/// The stop table's row height, points.
const ROW: f64 = 16.0;
/// The lowest a stop row's baseline may sit before the footer.
const TABLE_END: f64 = PAGE.1 - 64.0;
/// The longest design name used for files, in characters.
const MAX_FILE_NAME: usize = 64;
/// The machine file formats a production folder can hold.
const FORMATS: [&str; 3] = ["dst", "dsb", "exp"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stitch.worksheet",
            "Production Worksheet",
            [],
            None,
            "{path: (the PDF to write), hoop?: a hoop id of stitch.hoops.list (default: the document's hoop, else the smallest that fits), …the options of stitch.info (machine, spm, colorChangeS, trimS, marginMm, thresholds and the plan options)} write a one-page A4 PDF for the machine operator: the design name, the date, the design picture (the stitch preview), its size, stitches, colour changes, trims, hoop, machine, speed and estimated run time, top and bobbin thread, the design check's count, and the stop table (one row per thread block in sewing order: number, colour swatch, thread code and name (the file's, else the active chart's nearest), needle, stitches, length); type is set in the bundled Source Sans 3 → {path, bytes, stops, warnings (the PDF writer's)}",
            has_doc,
            worksheet_cmd
        ),
        cmd!(
            "stitch.exportProduction",
            "Export for Production",
            [],
            None,
            "{dir: (the folder: made if missing; a file there is an error), name?: (the files' base name, letters, digits, spaces, - _ and . only; default the design name), formats?: [dst|dsb|exp…] (default: the formats the machine's panel reads, else dst), worksheet?: true (also <name>-worksheet.pdf), …the options of stitch.worksheet} write the machine files and the worksheet into one folder, replacing files of the same names there → {dir, files: [{path, format: dst|dsb|exp|pdf, bytes}], findings (stitch.check's), warnings (the files' warnings)}",
            has_doc,
            export_cmd
        ),
    ]
}

/// A point type object at `at` (its baseline's start).
fn text(d: &mut Document, at: (f64, f64), s: &str, size: f64, bold: bool, rgb: [u8; 3]) -> Node {
    let style = CharStyle {
        size,
        font_style: if bold { "Semibold".into() } else { "Regular".into() },
        fill: Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])),
        stroke: Paint::None,
        ..CharStyle::default()
    };
    let mut t = TextObject::point(Point::new(at.0, at.1), s, style);
    t.para = super::super::create::new_type_para();
    super::super::typecmd::refresh_bounds(&mut t);
    Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)))
}

/// A rectangle filled with `fill` and stroked with `stroke` (0.5 pt) when given.
fn rect(d: &mut Document, r: Rect, fill: Option<[u8; 3]>, stroke: Option<[u8; 3]>) -> Node {
    let paint = |c: Option<[u8; 3]>| c.map_or(Paint::None, |[r, g, b]| Paint::solid(Color::rgb8(r, g, b)));
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(paint(fill), paint(stroke), 0.5))
}

/// `s` cut to `n` characters, with "…" when cut.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Today's date as `YYYY-MM-DD` (empty where there is no clock).
fn today() -> String {
    vectorcraft_doc::metadata::output_time()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// The design picture as PNG bytes and its pixel size.
fn picture(plan: &StitchPlan, fabric: [u8; 3], cmd: &str) -> Result<(Vec<u8>, u32, u32)> {
    let (w, h) =
        production::extents(plan).map_or((1.0, 1.0), |(x0, y0, x1, y1)| (f64::from(x1 - x0) / UNITS_PER_MM, f64::from(y1 - y0) / UNITS_PER_MM));
    let side = w.max(h).max(1.0) + 4.0;
    let scale = (PICTURE_PX / side).clamp(vectorcraft_stitch::MIN_SCALE, vectorcraft_stitch::MAX_SCALE).min(f64::from(MAX_SIDE) / side);
    let o = PreviewOptions { scale, fabric, mode: PreviewMode::Realistic, ..PreviewOptions::default() };
    let img = preview(plan, &o).map_err(|e| bad(cmd, e))?;
    let (pw, ph) = (img.width, img.height);
    let rgba = image::RgbaImage::from_raw(pw, ph, img.rgba).ok_or_else(|| bad(cmd, "the preview's pixels don't fit its size"))?;
    let mut png = Vec::new();
    rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| bad(cmd, e.to_string()))?;
    Ok((png, pw, ph))
}

/// One row of the stop table.
struct Stop {
    rgb: [u8; 3],
    code: String,
    name: String,
    needle: Option<usize>,
    stitches: usize,
    length_mm: f64,
}

fn stops(s: &Session, plan: &StitchPlan, needles: Option<u8>) -> Vec<Stop> {
    let chart = super::super::stitchthreads::active(s);
    let assigned = assign_needles(plan, needles);
    plan.blocks
        .iter()
        .map(|b| {
            let (mut stitches, mut units, mut prev) = (0usize, 0.0f64, None);
            for st in &b.steps {
                match st {
                    Step::Stitch(p) => {
                        stitches += 1;
                        if let Some(q) = prev {
                            units += p.dist(q);
                        }
                        prev = Some(*p);
                    }
                    _ => prev = None,
                }
            }
            let nearest = chart.as_ref().and_then(|(_, c)| c.nearest(b.thread.rgb).and_then(|m| c.threads.get(m.index).cloned()));
            let code = b.thread.code.clone().or_else(|| nearest.as_ref().map(|t| t.code.clone())).unwrap_or_default();
            let name =
                b.thread.name.clone().or_else(|| nearest.as_ref().map(|t| format!("{} {}", t.brand, t.name).trim().to_string())).unwrap_or_default();
            let needle = assigned.uses.iter().find(|u| u.rgb == b.thread.rgb).and_then(|u| u.needle);
            Stop { rgb: b.thread.rgb, code, name, needle, stitches, length_mm: units / UNITS_PER_MM }
        })
        .collect()
}

/// The worksheet for the options in `p`, as a document, with the number of stops.
fn worksheet_doc(s: &Session, p: &Value, cmd: &str) -> Result<(Document, usize)> {
    let prof = production::machine(p, cmd)?;
    let (plan, _) = production::plan_of(s, p, cmd)?;
    let st = stats(&plan);
    let est = estimates(&plan, &st, &prof, p, cmd)?;
    let margin = production::ranged(p, "marginMm", vectorcraft_stitch::DEFAULT_MARGIN_MM, (0.0, vectorcraft_stitch::MAX_MARGIN_MM), cmd)?;
    let src = &s.doc()?.doc;
    let fit = production::fit(src, &plan, &prof, margin);
    let hoop = match text_param(p, "hoop", cmd)? {
        Some(id) => Some(vectorcraft_stitch::hoop::find(id).ok_or_else(|| bad(cmd, format!("no hoop `{id}`: stitch.hoops.list lists them")))?),
        None => fit.as_ref().and_then(|f| f.chosen.clone().or_else(|| f.fitted.clone())),
    };
    let found: Vec<Finding> = findings(s, p, cmd)?;
    let (errors, warnings) = (found.iter().filter(|f| f.level == Level::Error).count(), found.iter().filter(|f| f.level == Level::Warning).count());
    let rows = stops(s, &plan, prof.needles);
    let fabric = src.stitch.fabric();
    let (png, pw, ph) = picture(&plan, fabric, cmd)?;

    let mut d = Document::new(PAGE.0, PAGE.1);
    d.title = plan.name.clone();
    let parent = d.layers.first().map(|l| l.id);
    let mut nodes: Vec<Node> = Vec::new();
    let (ink, dim, rule) = ([20, 20, 20], [110, 110, 110], [190, 190, 190]);

    nodes.push(text(&mut d, (MARGIN, 64.0), &cut(&plan.name, 48), 20.0, true, ink));
    let date = today();
    let sub = if date.is_empty() { format!("Production worksheet · {}", prof.id) } else { format!("Production worksheet · {date} · {}", prof.id) };
    nodes.push(text(&mut d, (MARGIN, 82.0), &sub, 9.0, false, dim));
    nodes.push(rect(&mut d, Rect::new(MARGIN, 92.0, PAGE.0 - MARGIN, 92.5), Some(rule), None));

    // The design picture, fitted into its box.
    let pic = Rect::new(MARGIN, 104.0, MARGIN + 260.0, 364.0);
    nodes.push(rect(&mut d, pic, Some(fabric), Some(rule)));
    let inner = pic.inset(-8.0);
    let k = (inner.width() / f64::from(pw)).min(inner.height() / f64::from(ph));
    let (iw, ih) = (f64::from(pw) * k, f64::from(ph) * k);
    let (ix, iy) = (inner.x0 + (inner.width() - iw) / 2.0, inner.y0 + (inner.height() - ih) / 2.0);
    let key = "stitch-worksheet-preview".to_string();
    d.images.insert(key.clone(), ImageBlob::png(png));
    let image = ImageObject {
        key,
        width: pw,
        height: ph,
        xf: Affine::translate((ix, iy)) * Affine::scale(k),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    nodes.push(Node::new(d.alloc_id(), NodeKind::Image(Box::new(image))));

    // The figures beside it.
    let colours = st.threads.len();
    let facts: Vec<(&str, String)> = vec![
        ("Size", format!("{:.1} × {:.1} mm", st.width_mm, st.height_mm)),
        ("Stitches", st.stitches.to_string()),
        ("Colours", format!("{colours} ({} changes, {} stops)", st.color_changes, rows.len())),
        ("Trims", st.trims.to_string()),
        ("Jumps", st.jumps.to_string()),
        ("Hoop", hoop.as_ref().map_or_else(|| "none fits".to_string(), |h| cut(&h.name, 30))),
        ("Machine", prof.id.to_string()),
        ("Speed", format!("{:.0} stitches/min", est.spm)),
        ("Run time", duration(est.run_time_s)),
        ("Top thread", format!("{:.1} m", est.top_mm / 1000.0)),
        ("Bobbin", format!("{:.1} m (estimate)", est.bobbin_mm / 1000.0)),
        ("Design check", format!("{errors} errors, {warnings} warnings")),
    ];
    let fx = MARGIN + 280.0;
    for (i, (label, value)) in facts.iter().enumerate() {
        let y = 116.0 + i as f64 * 20.0;
        nodes.push(text(&mut d, (fx, y), label, 9.0, false, dim));
        nodes.push(text(&mut d, (fx + 80.0, y), value, 10.0, false, ink));
    }

    // The stop table.
    let cols = [MARGIN, MARGIN + 24.0, MARGIN + 50.0, MARGIN + 120.0, MARGIN + 330.0, MARGIN + 380.0, MARGIN + 450.0];
    let head = ["#", "", "Code", "Thread", "Needle", "Stitches", "Length"];
    let top = 392.0;
    for (x, h) in cols.iter().zip(head) {
        nodes.push(text(&mut d, (*x, top), h, 9.0, true, dim));
    }
    nodes.push(rect(&mut d, Rect::new(MARGIN, top + 5.0, PAGE.0 - MARGIN, top + 5.5), Some(rule), None));
    let fits = (((TABLE_END - (top + ROW + 4.0)) / ROW).floor().max(1.0)) as usize;
    let shown = if rows.len() > fits { fits - 1 } else { rows.len() };
    for (i, r) in rows.iter().take(shown).enumerate() {
        let y = top + ROW + 4.0 + i as f64 * ROW;
        let [cr, cg, cb] = r.rgb;
        let cells = [
            (i + 1).to_string(),
            String::new(),
            if r.code.is_empty() { format!("#{cr:02x}{cg:02x}{cb:02x}") } else { cut(&r.code, 12) },
            cut(&r.name, 38),
            r.needle.map_or_else(|| "swap".to_string(), |n| n.to_string()),
            r.stitches.to_string(),
            format!("{:.1} m", r.length_mm / 1000.0),
        ];
        for (x, c) in cols.iter().zip(cells) {
            if !c.is_empty() {
                nodes.push(text(&mut d, (*x, y), &c, 10.0, false, ink));
            }
        }
        nodes.push(rect(&mut d, Rect::new(cols[1], y - 9.0, cols[1] + 16.0, y + 1.0), Some(r.rgb), Some(dim)));
    }
    if shown < rows.len() {
        let y = top + ROW + 4.0 + shown as f64 * ROW;
        nodes.push(text(&mut d, (MARGIN, y), &format!("… and {} more stops (stitch.needles lists them all)", rows.len() - shown), 9.0, false, dim));
    }
    let note = format!(
        "Estimates: bobbin about half the top thread; run time at {:.0} stitches/min plus time for each colour change and trim. Lengths are stitched lengths.",
        est.spm
    );
    nodes.push(text(&mut d, (MARGIN, PAGE.1 - 40.0), &note, 7.5, false, dim));

    for n in nodes {
        d.insert(parent, usize::MAX, n).map_err(|e| bad(cmd, e.to_string()))?;
    }
    Ok((d, rows.len()))
}

/// The worksheet as PDF bytes, with the writer's warnings and the number of stops.
fn worksheet_pdf(s: &Session, p: &Value, cmd: &str) -> Result<(Vec<u8>, Vec<String>, usize)> {
    let (doc, stops) = worksheet_doc(s, p, cmd)?;
    let (bytes, warnings) = fileio::pdf::encode(cmd, &doc, &json!({}))?;
    Ok((bytes, warnings, stops))
}

fn worksheet_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let path = text_param(p, "path", WORKSHEET)?.ok_or_else(|| bad(WORKSHEET, "give path: the PDF to write"))?;
    let (bytes, warnings, stops) = worksheet_pdf(s, p, WORKSHEET)?;
    fileio::write_file(path, &bytes)?;
    Ok(json!({"path": path, "bytes": bytes.len(), "stops": stops, "warnings": warnings}))
}

/// A file base name from `name`: letters, digits, spaces, `-`, `_` and `.`; never empty, never
/// starting with a dot, at most [`MAX_FILE_NAME`] characters. `given`: the user's, which is refused
/// rather than changed when it holds anything else.
fn file_name(name: &str, given: bool) -> Result<String> {
    let ok = |c: char| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.');
    if given && (name.chars().any(|c| !ok(c)) || name.contains("..") || name.trim().is_empty() || name.starts_with('.')) {
        return Err(bad(EXPORT, format!("name `{}`: letters, digits, spaces, - _ and . only, not starting with a dot", cut(name, 40))));
    }
    let clean: String = name.chars().map(|c| if ok(c) { c } else { '_' }).take(MAX_FILE_NAME).collect();
    let clean = clean.trim().trim_start_matches('.').replace("..", "_");
    Ok(if clean.is_empty() { "design".into() } else { clean })
}

fn export_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = text_param(p, "dir", EXPORT)?.ok_or_else(|| bad(EXPORT, "give dir: the folder to write into"))?;
    let prof = production::machine(p, EXPORT)?;
    let formats: Vec<&str> = match p.get("formats") {
        None | Some(Value::Null) => {
            if prof.formats.is_empty() {
                vec!["dst"]
            } else {
                prof.formats.to_vec()
            }
        }
        Some(Value::Array(a)) => {
            let mut v: Vec<&str> = vec![];
            for f in a {
                let f = f.as_str().and_then(|f| FORMATS.iter().find(|x| **x == f).copied());
                let f = f.ok_or_else(|| bad(EXPORT, "formats lists dst, dsb or exp"))?;
                if !v.contains(&f) {
                    v.push(f);
                }
            }
            v
        }
        Some(_) => return Err(bad(EXPORT, "formats must be a list: [dst, dsb, exp]")),
    };
    let with_sheet = switch_or(p, "worksheet", true, EXPORT)?;
    if formats.is_empty() && !with_sheet {
        return Err(bad(EXPORT, "nothing to write: give formats or worksheet"));
    }
    // Everything is made before anything is written, so an error leaves the folder as it was.
    let selection = super::selected(s, p, EXPORT)?;
    let doc = &s.doc()?.doc;
    let mut out: Vec<(String, &str, Vec<u8>)> = vec![];
    let mut warnings: Vec<String> = vec![];
    let mut design = String::new();
    for f in &formats {
        let mut q = p.clone();
        if let Some(o) = q.as_object_mut() {
            o.insert("format".into(), json!(f));
        }
        let (plan, written) = stitchfile::written(EXPORT, doc, &q, f, selection.clone())?;
        design = plan.name.clone();
        for w in written.warnings {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        out.push((String::new(), f, written.bytes));
    }
    if with_sheet {
        let (bytes, w, _) = worksheet_pdf(s, p, EXPORT)?;
        warnings.extend(w);
        if design.is_empty() {
            design = production::plan_of(s, p, EXPORT)?.0.name;
        }
        out.push((String::new(), "pdf", bytes));
    }
    let base = match text_param(p, "name", EXPORT)? {
        Some(n) => file_name(n, true)?,
        None => file_name(&design, false)?,
    };
    let found = findings(s, p, EXPORT)?;
    let root = std::path::Path::new(dir);
    if root.exists() && !root.is_dir() {
        return Err(bad(EXPORT, format!("{dir} is a file, not a folder")));
    }
    fileio::create_dir(dir)?;
    let mut files = vec![];
    for (_, format, bytes) in &mut out {
        let file = if *format == "pdf" { format!("{base}-worksheet.pdf") } else { format!("{base}.{format}") };
        let path = root.join(&file).to_string_lossy().into_owned();
        fileio::write_file(&path, bytes)?;
        files.push(json!({"path": path, "format": format, "bytes": bytes.len()}));
    }
    Ok(json!({
        "dir": dir,
        "files": files,
        "findings": found.iter().map(production::finding_json).collect::<Vec<_>>(),
        "warnings": warnings,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn calibration() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        s.execute("stitch.calibration", &json!({})).unwrap();
        s
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("vc-stitch-prod-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_worksheet_is_a_one_page_pdf_with_the_figures() {
        let s = calibration();
        let (doc, stops) = super::worksheet_doc(&s, &json!({"machine": "bai_mirror", "name": "Calibration"}), "t").unwrap();
        assert_eq!(stops, 3);
        assert_eq!(doc.images.len(), 1);
        let texts: Vec<String> = doc
            .layers
            .iter()
            .flat_map(|l| l.children().into_iter().flatten())
            .filter_map(|n| if let vectorcraft_doc::NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None })
            .collect();
        for want in ["Calibration", "Stitches", "Run time", "Bobbin", "Needle", "Rectangle 130 × 180 mm"] {
            assert!(texts.iter().any(|t| t.contains(want)), "{want} in {texts:?}");
        }
        let mut s = s;
        let dir = tmp("sheet");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.pdf");
        let r = s.execute("stitch.worksheet", &json!({"path": path.to_string_lossy(), "machine": "bai_mirror"})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "{r}");
        assert_eq!(r["stops"], 3);
        assert!(s.execute("stitch.worksheet", &json!({})).is_err());
        assert!(s.execute("stitch.worksheet", &json!({"path": path.to_string_lossy(), "hoop": "nope"})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_production_folder_holds_the_machine_files_and_the_worksheet() {
        let mut s = calibration();
        let dir = tmp("folder");
        let d = dir.to_string_lossy().to_string();
        let r = s.execute("stitch.exportProduction", &json!({"dir": d, "machine": "bai_mirror", "name": "cal"})).unwrap();
        let names: Vec<String> =
            r["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().rsplit('/').next().unwrap().to_string()).collect();
        assert_eq!(names, ["cal.dst", "cal.dsb", "cal-worksheet.pdf"], "{r}");
        for f in r["files"].as_array().unwrap() {
            assert_eq!(std::fs::metadata(f["path"].as_str().unwrap()).unwrap().len(), f["bytes"].as_u64().unwrap());
        }
        assert!(r["findings"].as_array().unwrap().iter().any(|f| f["code"] == "hoop"));
        // The same again replaces them; EXP only without the worksheet; a bad name or format is refused.
        let r = s.execute("stitch.exportProduction", &json!({"dir": d, "formats": ["exp"], "worksheet": false})).unwrap();
        assert_eq!(r["files"].as_array().unwrap().len(), 1);
        for bad in [
            json!({"dir": d, "name": "../escape"}),
            json!({"dir": d, "name": "a/b"}),
            json!({"dir": d, "name": ".hidden"}),
            json!({"dir": d, "formats": ["pes"]}),
            json!({"dir": d, "formats": [], "worksheet": false}),
            json!({}),
        ] {
            assert!(s.execute("stitch.exportProduction", &bad).is_err(), "{bad}");
        }
        // A file where the folder should be.
        let file = dir.join("cal.dst").to_string_lossy().to_string();
        assert!(s.execute("stitch.exportProduction", &json!({"dir": file})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_names_are_cleaned_or_refused() {
        assert_eq!(super::file_name("My design: v2", false).unwrap(), "My design_ v2");
        assert_eq!(super::file_name("..", false).unwrap(), "design");
        assert_eq!(super::file_name("", false).unwrap(), "design");
        assert!(super::file_name("a\\b", true).is_err());
        assert_eq!(super::file_name("Logo 2026-11.v1", true).unwrap(), "Logo 2026-11.v1");
    }
}
