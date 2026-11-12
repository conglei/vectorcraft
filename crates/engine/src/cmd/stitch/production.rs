//! Embroidery production: the hoop (`stitch.hoops.list`, `stitch.hoop.set`, `stitch.hoop.get`,
//! `stitch.hoop.fit`, `stitch.hoop.center`), the estimates `stitch.info` adds (bobbin, run time,
//! needles) and the design checker (`stitch.check`). The worksheet and the production folder are
//! in [`super::worksheet`].
//!
//! The hoop's centre on the page is the first artboard's centre; `stitch.hoop.fit` reports the
//! design's offset from it, and `stitch.hoop.center` moves the art onto it.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_geom::{Affine, Point};
use vectorcraft_stitch::hoop as hoops;
use vectorcraft_stitch::{
    CELL_RANGE, CheckLimits, DEFAULT_MARGIN_MM, DENSITY_RANGE, Finding, Hoop, HoopShape, LONG_RANGE, Level, MAX_MARGIN_MM, MAX_TRIMS_LIMIT,
    MachineProfile, PT_PER_MM, Pt, SHORT_RANGE, Stats, StitchPlan, UNITS_PER_MM, assign_needles, check_plan, profile,
};

use super::super::fileio::stitchfile;
use super::super::{CommandSpec, Result, Session, bad, cmd, has_doc, number_or, switch_or, text_param};

const LIST: &str = "stitch.hoops.list";
const SET: &str = "stitch.hoop.set";
const FIT: &str = "stitch.hoop.fit";
const CENTER: &str = "stitch.hoop.center";
const CHECK: &str = "stitch.check";

/// Bobbin thread per unit of top thread: thread use splits about 2/3 top, 1/3 bobbin
/// (<https://www.madeira.com/en-in/embroidery-supplies/industrial-embroidery-threads>), so the
/// bobbin uses about half the top thread's length.
pub(crate) const BOBBIN_RATIO: f64 = 0.5;
/// The speed assumed for a machine whose profile names none, stitches per minute.
pub(crate) const DEFAULT_SPM: f64 = 800.0;
/// Seconds allowed per colour change (a needle change and the slow start after it): an assumption,
/// to be measured at the stitch-out.
pub(crate) const COLOR_CHANGE_S: f64 = 5.0;
/// Seconds allowed per trim: an assumption, to be measured at the stitch-out.
pub(crate) const TRIM_S: f64 = 3.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stitch.hoops.list",
            "Hoops",
            [],
            None,
            "{machine?: commercial15 (default) | bai_mirror | generic} the built-in hoops and frames that fit the machine's field, smallest first: generic sizes by their inner dimensions, no maker's catalogue → {machine, field: {widthMm, heightMm}|null, hoops: [{id, name, widthMm, heightMm, shape: rectangle|round (round: the ellipse inside width × height), cap (a cap frame: never chosen by stitch.hoop.fit)}], chosen: the document's hoop (stitch.hoop.get)|null}",
            has_doc,
            list
        ),
        cmd!(
            query "stitch.hoop.get",
            "Hoop",
            [],
            None,
            "{} the document's hoop → {hoop: {id (null: custom), name, widthMm, heightMm, shape: rectangle|round, cap}|null, center: {x, y} (where the hoop's centre is on the page, document units: the first artboard's centre)}",
            has_doc,
            get
        ),
        cmd!(
            "stitch.hoop.set",
            "Set Hoop",
            [],
            None,
            "{id?: a hoop of stitch.hoops.list | widthMm?, heightMm?: (a custom hoop, mm, 10–2000), name?: (a custom hoop's name), shape?: rectangle (default) | round, clear?: false (no hoop)} the hoop the design is sewn in, kept in the document; stitch.check and the worksheet use it; one undo step when it changes → {hoop: (as stitch.hoop.get)|null}",
            has_doc,
            set
        ),
        cmd!(
            query "stitch.hoop.fit",
            "Fit Hoop",
            [],
            None,
            "{marginMm?: 10 (clearance from the hoop's edge, 0–100), …the plan options of stitch.info} the smallest built-in hoop by area (cap frames left out) within the machine's field that the stitches fit, centred, with the margin; and whether they fit the document's hoop → {widthMm, heightMm (the stitches' extents), fits (some hoop holds them), hoop: the smallest that does|null, chosen: the document's hoop|null, fitsChosen (centred)|null, inPlace (where the stitches are now, inside the chosen or else the fitted hoop centred on the first artboard's centre)|null, offsetMm: {x, y} (the stitches' centre from the hoop's centre, the first artboard's centre, mm, y down)}",
            has_doc,
            fit_cmd
        ),
        cmd!(
            "stitch.hoop.center",
            "Center Design in Hoop",
            ["Embroidery"],
            None,
            "{…the plan options of stitch.info} move the art so the stitches' centre is the hoop's centre (the first artboard's centre); with selectedOnly the selection moves, else every object on an unlocked, visible layer; one undo step, none when it is already centred → {moved, dx, dy (document units)}",
            has_doc,
            center_cmd
        ),
        cmd!(
            query "stitch.check",
            "Design Check",
            [],
            None,
            "{marginMm?: 10 (the hoop fit's clearance, 0–100), thresholds?: {shortMm?: 0.3 (stitches shorter are reported, 0–5; the export drops those under its minStitch, 0.3 by default, so these come from a lower minStitch or an opened file), longMm?: 7 (stitches longer are reported, 1–12.7), densityCellMm?: 2 (the density grid's cell, 0.5–10), maxDensity?: 4 (needle points per mm² in a cell, 0.1–100), maxTrims?: 50}, …the plan options of stitch.info} check the stitch plan an export would write → {findings: [{level: info|warning|error, code: empty|overMaxStitch|shortStitches|longStitches|density|trims|colors|hoopTooSmall|noHoopFits|fieldTooSmall|offCenter|hoop, message, count?, where?: {x, y} (document units: the first case, or the densest cell's centre)}] (errors first), errors, warnings}; a document with nothing to sew is an `empty` error finding, not a command error",
            has_doc,
            check_cmd
        ),
    ]
}

/// The machine `p` names (`commercial15` when not given).
pub(crate) fn machine(p: &Value, cmd: &str) -> Result<MachineProfile> {
    let id = text_param(p, "machine", cmd)?.unwrap_or("commercial15");
    profile(id).ok_or_else(|| bad(cmd, format!("machine `{id}`: generic, commercial15 or bai_mirror")))
}

/// The stitch plan an export with the options in `p` would write, and its warnings.
pub(crate) fn plan_of(s: &Session, p: &Value, cmd: &str) -> Result<(StitchPlan, Vec<String>)> {
    let format = text_param(p, "format", cmd)?.unwrap_or("dst");
    let selection = super::selected(s, p, cmd)?;
    stitchfile::plan(cmd, &s.doc()?.doc, p, format, selection)
}

/// The number `key` in `p` (default `default`), finite and within `lo..=hi`.
pub(crate) fn ranged(p: &Value, key: &str, default: f64, (lo, hi): (f64, f64), cmd: &str) -> Result<f64> {
    let v = number_or(p, key, default, cmd)?;
    if !(v.is_finite() && (lo..=hi).contains(&v)) {
        return Err(bad(cmd, format!("{key} must be {lo}–{hi}, not {v}")));
    }
    Ok(v)
}

fn margin(p: &Value, cmd: &str) -> Result<f64> {
    ranged(p, "marginMm", DEFAULT_MARGIN_MM, (0.0, MAX_MARGIN_MM), cmd)
}

/// The document's hoop, if set and usable.
pub(crate) fn chosen(doc: &Document) -> Option<Hoop> {
    doc.stitch.hoop().cloned()
}

/// Where the hoop's centre is on the page: the first artboard's centre (the origin when there is
/// none).
pub(crate) fn hoop_center(doc: &Document) -> Point {
    doc.artboards.first().map_or(Point::ZERO, |a| a.rect.center())
}

pub(crate) fn hoop_json(h: &Hoop) -> Value {
    json!({"id": h.id, "name": h.name, "widthMm": h.width_mm, "heightMm": h.height_mm, "shape": h.shape.id(), "cap": h.cap})
}

fn chosen_json(doc: &Document) -> Value {
    chosen(doc).as_ref().map_or(Value::Null, hoop_json)
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let prof = machine(p, LIST)?;
    let list: Vec<Value> = hoops::for_field(prof.field_mm).iter().map(hoop_json).collect();
    let field = prof.field_mm.map_or(Value::Null, |(w, h)| json!({"widthMm": w, "heightMm": h}));
    Ok(json!({"machine": prof.id, "field": field, "hoops": list, "chosen": chosen_json(&s.doc()?.doc)}))
}

fn get(s: &mut Session, _: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let c = hoop_center(doc);
    Ok(json!({"hoop": chosen_json(doc), "center": {"x": c.x, "y": c.y}}))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let now = s.doc()?.doc.stitch.clone();
    let mut next = now.clone();
    if switch_or(p, "clear", false, SET)? {
        next.hoop = None;
    } else {
        let id = text_param(p, "id", SET)?;
        let has_size = p.get("widthMm").is_some() || p.get("heightMm").is_some();
        next.hoop = Some(match (id, has_size) {
            (Some(_), true) => return Err(bad(SET, "give id or widthMm and heightMm, not both")),
            (Some(id), false) => hoops::find(id).ok_or_else(|| bad(SET, format!("no hoop `{id}`: stitch.hoops.list lists them")))?,
            (None, true) => {
                let w = number_or(p, "widthMm", f64::NAN, SET)?;
                let h = number_or(p, "heightMm", f64::NAN, SET)?;
                let shape = match text_param(p, "shape", SET)? {
                    None => HoopShape::Rectangle,
                    Some(v) => HoopShape::parse(v).ok_or_else(|| bad(SET, format!("shape `{v}`: rectangle or round")))?,
                };
                let name = text_param(p, "name", SET)?.unwrap_or("");
                if name.chars().count() > vectorcraft_doc::stitch_design::MAX_HOOP_NAME {
                    return Err(bad(SET, "the name is too long"));
                }
                hoops::custom(name, w, h, shape).map_err(|e| bad(SET, e))?
            }
            (None, false) => return Err(bad(SET, "give id, widthMm and heightMm, or clear")),
        });
    }
    if next != now {
        s.edit("Set Hoop", |d, _| {
            d.stitch = next;
            Ok(())
        })?;
    }
    Ok(json!({"hoop": chosen_json(&s.doc()?.doc)}))
}

/// The stitches' extents in plan units: (left, top, right, bottom).
pub(crate) fn extents(plan: &StitchPlan) -> Option<(i32, i32, i32, i32)> {
    let mut b: Option<(i32, i32, i32, i32)> = None;
    for s in plan.blocks.iter().flat_map(|b| b.steps.iter()) {
        if let vectorcraft_stitch::Step::Stitch(p) = s {
            b = Some(match b {
                None => (p.x, p.y, p.x, p.y),
                Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
            });
        }
    }
    b
}

/// How a plan sits in the hoops: the hoop fit of `stitch.hoop.fit`.
pub(crate) struct Fit {
    pub width_mm: f64,
    pub height_mm: f64,
    /// The smallest library hoop that holds the stitches, centred.
    pub fitted: Option<Hoop>,
    /// The document's hoop.
    pub chosen: Option<Hoop>,
    pub fits_chosen: Option<bool>,
    /// Whether the stitches, where they are, lie in the hoop checked (chosen, else fitted).
    pub in_place: Option<bool>,
    /// The stitches' centre from the hoop's centre, mm.
    pub offset_mm: (f64, f64),
}

pub(crate) fn fit(doc: &Document, plan: &StitchPlan, prof: &MachineProfile, margin: f64) -> Option<Fit> {
    let (x0, y0, x1, y1) = extents(plan)?;
    let mm = |v: i32| f64::from(v) / UNITS_PER_MM;
    let (w, h) = (mm(x1) - mm(x0), mm(y1) - mm(y0));
    let fitted = hoops::fit((w, h), margin, prof.field_mm);
    let chosen = chosen(doc);
    let fits_chosen = chosen.as_ref().map(|c| hoops::fits((w, h), c, margin));
    let c = hoop_center(doc);
    let (cx, cy) = (c.x / PT_PER_MM, c.y / PT_PER_MM);
    let rel = (mm(x0) - cx, mm(y0) - cy, mm(x1) - cx, mm(y1) - cy);
    let offset_mm = ((rel.0 + rel.2) / 2.0, (rel.1 + rel.3) / 2.0);
    // The hoop the stitches are placed in: the chosen one, else the fitted one.
    let in_place = chosen.as_ref().or(fitted.as_ref()).map(|hoop| hoop.holds(rel, margin));
    Some(Fit { width_mm: w, height_mm: h, fitted, chosen, fits_chosen, in_place, offset_mm })
}

fn round(v: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    // Adding 0 turns -0 into 0.
    (v * k).round() / k + 0.0
}

fn fit_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let m = margin(p, FIT)?;
    let prof = machine(p, FIT)?;
    let (plan, _) = plan_of(s, p, FIT)?;
    let Some(f) = fit(&s.doc()?.doc, &plan, &prof, m) else { return Err(bad(FIT, "the design has no stitches")) };
    Ok(json!({
        "widthMm": round(f.width_mm, 1),
        "heightMm": round(f.height_mm, 1),
        "fits": f.fitted.is_some(),
        "hoop": f.fitted.as_ref().map_or(Value::Null, hoop_json),
        "chosen": chosen_json(&s.doc()?.doc),
        "fitsChosen": f.fits_chosen,
        "inPlace": f.in_place,
        "offsetMm": {"x": round(f.offset_mm.0, 1), "y": round(f.offset_mm.1, 1)},
    }))
}

/// The objects `stitch.hoop.center` moves: the selection with `selectedOnly`, else every object on
/// an unlocked, visible layer.
fn movable(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    if let Some(sel) = super::selected(s, p, CENTER)? {
        return Ok(sel);
    }
    let doc = &s.doc()?.doc;
    Ok(doc
        .layers
        .iter()
        .filter(|l| l.visible && !l.locked)
        .flat_map(|l| l.children().into_iter().flatten())
        .filter(|n| !n.locked)
        .map(|n| n.id)
        .collect())
}

fn center_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (plan, _) = plan_of(s, p, CENTER)?;
    let Some((x0, y0, x1, y1)) = extents(&plan) else { return Err(bad(CENTER, "the design has no stitches")) };
    let doc = &s.doc()?.doc;
    let c = hoop_center(doc);
    // The stitches' centre in document units.
    let to_pt = |v: i32| f64::from(v) / UNITS_PER_MM * PT_PER_MM;
    let (dx, dy) = (c.x - (to_pt(x0) + to_pt(x1)) / 2.0, c.y - (to_pt(y0) + to_pt(y1)) / 2.0);
    // Within half a stitch unit: already centred.
    let tolerance = PT_PER_MM / UNITS_PER_MM / 2.0;
    if dx.abs() <= tolerance && dy.abs() <= tolerance {
        return Ok(json!({"moved": false, "dx": 0.0, "dy": 0.0}));
    }
    let ids = movable(s, p)?;
    if ids.is_empty() {
        return Err(bad(CENTER, "nothing can move: the layers are locked or hidden"));
    }
    super::super::object::apply_transform(s, "Center Design in Hoop", ids, Affine::translate((dx, dy)), &json!({}), false)?;
    Ok(json!({"moved": true, "dx": round(dx, 3), "dy": round(dy, 3)}))
}

/// The checker thresholds in `p`'s `thresholds`.
fn limits(p: &Value, cmd: &str) -> Result<CheckLimits> {
    let d = CheckLimits::default();
    let t = match p.get("thresholds") {
        None | Some(Value::Null) => return Ok(d),
        Some(t @ Value::Object(o)) => {
            const KEYS: [&str; 5] = ["shortMm", "longMm", "densityCellMm", "maxDensity", "maxTrims"];
            if let Some(k) = o.keys().find(|k| !KEYS.contains(&k.as_str())) {
                return Err(bad(cmd, format!("unknown parameter `thresholds.{k}`; it takes: {}", KEYS.join(", "))));
            }
            t
        }
        Some(t) => return Err(bad(cmd, format!("thresholds must be an object, not {t}"))),
    };
    let trims = ranged(t, "maxTrims", d.max_trims as f64, (0.0, MAX_TRIMS_LIMIT as f64), cmd)?;
    Ok(CheckLimits {
        short_mm: ranged(t, "shortMm", d.short_mm, SHORT_RANGE, cmd)?,
        long_mm: ranged(t, "longMm", d.long_mm, LONG_RANGE, cmd)?,
        cell_mm: ranged(t, "densityCellMm", d.cell_mm, CELL_RANGE, cmd)?,
        max_density: ranged(t, "maxDensity", d.max_density, DENSITY_RANGE, cmd)?,
        // Finite, whole-number range checked above.
        max_trims: trims.round() as usize,
    })
}

/// The hoop findings for `plan`.
fn hoop_findings(doc: &Document, plan: &StitchPlan, prof: &MachineProfile, margin: f64) -> Vec<Finding> {
    let Some(f) = fit(doc, plan, prof, margin) else { return vec![] };
    let size = format!("{:.1} × {:.1} mm", f.width_mm, f.height_mm);
    let mut out = vec![];
    let finding = |level, code, message: String| Finding { level, code, message, count: None, at: None };
    if let Some((fw, fh)) = prof.field_mm
        && (f.width_mm > fw || f.height_mm > fh)
    {
        out.push(finding(Level::Error, "fieldTooSmall", format!("the design ({size}) is larger than the machine's {fw} × {fh} mm field")));
    }
    match (&f.chosen, f.fits_chosen) {
        (Some(h), Some(false)) => {
            out.push(finding(Level::Error, "hoopTooSmall", format!("the design ({size}) does not fit the {} with a {margin} mm margin", h.name)))
        }
        (Some(h), _) => out.push(finding(Level::Info, "hoop", format!("the design ({size}) fits the {}", h.name))),
        (None, _) => match &f.fitted {
            Some(h) => out.push(finding(Level::Info, "hoop", format!("the design ({size}) fits the {} (the smallest that holds it)", h.name))),
            None => {
                out.push(finding(Level::Error, "noHoopFits", format!("no hoop for this machine holds the design ({size}) with a {margin} mm margin")))
            }
        },
    }
    if f.in_place == Some(false) && f.fits_chosen != Some(false) && (f.chosen.is_some() || f.fitted.is_some()) {
        let (x, y) = f.offset_mm;
        out.push(finding(
            Level::Info,
            "offCenter",
            format!("the design is {x:.1}, {y:.1} mm from the hoop's centre and over its edge there: Center Design in Hoop moves it"),
        ));
    }
    out
}

/// The design checker's findings for `p` (`marginMm`, `thresholds`, `machine` and the plan
/// options), errors first. A document with nothing to sew gives an `empty` finding.
pub(crate) fn findings(s: &Session, p: &Value, cmd: &str) -> Result<Vec<Finding>> {
    let m = margin(p, cmd)?;
    let l = limits(p, cmd)?;
    let prof = machine(p, cmd)?;
    let plan = match plan_of(s, p, cmd) {
        Ok((plan, _)) => plan,
        // Nothing to sew is a finding here (the planner's message names it).
        Err(e) if e.to_string().contains("nothing to stitch") => StitchPlan::default(),
        Err(e) => return Err(e),
    };
    let mut out = check_plan(&plan, &prof, &l);
    out.extend(hoop_findings(&s.doc()?.doc, &plan, &prof, m));
    out.sort_by_key(|f| std::cmp::Reverse(f.level));
    Ok(out)
}

/// A plan point in document units.
fn doc_xy(p: Pt) -> Value {
    let (x, y) = p.to_mm();
    json!({"x": round(x * PT_PER_MM, 3), "y": round(y * PT_PER_MM, 3)})
}

pub(crate) fn finding_json(f: &Finding) -> Value {
    let mut v = json!({"level": f.level.id(), "code": f.code, "message": f.message});
    if let Some(n) = f.count {
        v["count"] = json!(n);
    }
    if let Some(p) = f.at {
        v["where"] = doc_xy(p);
    }
    v
}

fn check_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let f = findings(s, p, CHECK)?;
    let n = |l: Level| f.iter().filter(|x| x.level == l).count();
    Ok(json!({"findings": f.iter().map(finding_json).collect::<Vec<_>>(), "errors": n(Level::Error), "warnings": n(Level::Warning)}))
}

/// The run-time and thread estimates of a plan.
pub(crate) struct Estimates {
    pub spm: f64,
    pub run_time_s: f64,
    pub top_mm: f64,
    pub bobbin_mm: f64,
    /// The needle of each thread colour of `stats.threads`, in order (`None`: past the needles).
    pub needles: Vec<Option<usize>>,
}

/// The estimate options in `p` (`spm`, `colorChangeS`, `trimS`) for `plan`.
pub(crate) fn estimates(plan: &StitchPlan, st: &Stats, prof: &MachineProfile, p: &Value, cmd: &str) -> Result<Estimates> {
    let spm = ranged(p, "spm", prof.max_spm.map_or(DEFAULT_SPM, f64::from), (100.0, 3000.0), cmd)?;
    let cc = ranged(p, "colorChangeS", COLOR_CHANGE_S, (0.0, 600.0), cmd)?;
    let trim = ranged(p, "trimS", TRIM_S, (0.0, 600.0), cmd)?;
    // Jumps move the frame at about the sewing speed.
    let moves = (st.stitches + st.jumps) as f64;
    let run_time_s = moves / spm * 60.0 + st.color_changes as f64 * cc + st.trims as f64 * trim;
    let top_mm: f64 = st.threads.iter().map(|t| t.length_mm).sum();
    let assigned = assign_needles(plan, prof.needles);
    let needles = st.threads.iter().map(|t| assigned.uses.iter().find(|u| u.rgb == t.rgb).and_then(|u| u.needle)).collect();
    Ok(Estimates { spm, run_time_s, top_mm, bobbin_mm: top_mm * BOBBIN_RATIO, needles })
}

/// `seconds` as `1 h 02 min`, `12 min 05 s` or `40 s`.
pub(crate) fn duration(seconds: f64) -> String {
    // Finite and non-negative by construction; capped so the cast is exact.
    let s = seconds.round().clamp(0.0, 1e9) as u64;
    match (s / 3600, (s % 3600) / 60, s % 60) {
        (0, 0, sec) => format!("{sec} s"),
        (0, m, sec) => format!("{m} min {sec:02} s"),
        (h, m, _) => format!("{h} h {m:02} min"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::Session;

    fn calibration() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        s.execute("stitch.calibration", &json!({})).unwrap();
        s
    }

    fn codes(r: &Value) -> Vec<String> {
        r["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn hoops_are_listed_for_the_machine_and_set_custom_or_by_id() {
        let mut s = calibration();
        let r = s.execute("stitch.hoops.list", &json!({"machine": "bai_mirror"})).unwrap();
        assert_eq!(r["hoops"].as_array().unwrap().len(), 10);
        assert_eq!(r["field"], json!({"widthMm": 500.0, "heightMm": 350.0}));
        assert_eq!(r["chosen"], Value::Null);
        let undo = s.active().unwrap().history.undo.len();
        let r = s.execute("stitch.hoop.set", &json!({"id": "square-200"})).unwrap();
        assert_eq!(r["hoop"]["widthMm"], 200.0);
        assert_eq!(r["hoop"]["id"], "square-200");
        assert_eq!(s.active().unwrap().history.undo.len(), undo + 1);
        let g = s.execute("stitch.hoop.get", &json!({})).unwrap();
        assert_eq!(g["center"], json!({"x": 300.0, "y": 200.0}));
        assert_eq!(g["hoop"]["shape"], "rectangle");
        // The same again changes nothing and adds no undo step.
        s.execute("stitch.hoop.set", &json!({"id": "square-200"})).unwrap();
        assert_eq!(s.active().unwrap().history.undo.len(), undo + 1);
        let r = s.execute("stitch.hoop.set", &json!({"widthMm": 150, "heightMm": 120, "shape": "round"})).unwrap();
        assert_eq!(r["hoop"]["id"], Value::Null);
        assert_eq!(r["hoop"]["shape"], "round");
        assert_eq!(r["hoop"]["name"], "Custom 150 × 120 mm");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("stitch.hoop.get", &json!({})).unwrap()["hoop"]["id"], "square-200");
        assert_eq!(s.execute("stitch.hoop.set", &json!({"clear": true})).unwrap()["hoop"], Value::Null);
        for bad in [
            json!({"id": "nope"}),
            json!({"widthMm": 5, "heightMm": 100}),
            json!({"widthMm": 100}),
            json!({"id": "square-100", "widthMm": 100}),
            json!({"widthMm": 100, "heightMm": 100, "shape": "star"}),
            json!({"id": "square-100", "center": [1, 2]}),
            json!({}),
        ] {
            assert!(s.execute("stitch.hoop.set", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_calibration_design_fits_and_centres_in_its_hoop() {
        let mut s = calibration();
        let r = s.execute("stitch.hoop.fit", &json!({"machine": "bai_mirror"})).unwrap();
        // About 100 × 100 mm with lock stitches: with 10 mm each side, 130 × 180 is the smallest.
        assert_eq!(r["hoop"]["id"], "rect-130x180", "{r}");
        assert!(r["fits"].as_bool().unwrap());
        assert_eq!(r["chosen"], Value::Null);
        // The design sits at the page's top-left, off the artboard's centre: not in place.
        assert_eq!(r["inPlace"], false, "{r}");
        let c = s.execute("stitch.hoop.center", &json!({})).unwrap();
        assert_eq!(c["moved"], true);
        let r = s.execute("stitch.hoop.fit", &json!({})).unwrap();
        assert_eq!(r["inPlace"], true, "{r}");
        assert!(r["offsetMm"]["x"].as_f64().unwrap().abs() <= 0.1 && r["offsetMm"]["y"].as_f64().unwrap().abs() <= 0.1, "{r}");
        // Centred already: no move and no undo step.
        let undo = s.active().unwrap().history.undo.len();
        assert_eq!(s.execute("stitch.hoop.center", &json!({})).unwrap()["moved"], false);
        assert_eq!(s.active().unwrap().history.undo.len(), undo);
        // A chosen hoop too small is reported.
        s.execute("stitch.hoop.set", &json!({"id": "square-100"})).unwrap();
        let r = s.execute("stitch.hoop.fit", &json!({})).unwrap();
        assert_eq!((r["fitsChosen"].clone(), r["inPlace"].clone()), (json!(false), json!(false)));
        assert!(s.execute("stitch.hoop.fit", &json!({"marginMm": 101})).is_err());
    }

    #[test]
    fn the_checker_reports_hoop_density_and_an_empty_design() {
        let mut s = calibration();
        let r = s.execute("stitch.check", &json!({"machine": "bai_mirror"})).unwrap();
        assert_eq!(r["errors"], 0, "{r}");
        assert!(codes(&r).contains(&"hoop".to_string()), "{r}");
        assert!(codes(&r).contains(&"colors".to_string()), "{r}");
        s.execute("stitch.hoop.set", &json!({"id": "square-100"})).unwrap();
        let r = s.execute("stitch.check", &json!({})).unwrap();
        assert_eq!(codes(&r)[0], "hoopTooSmall", "{r}");
        assert_eq!(r["findings"][0]["level"], "error");
        // A strict density limit flags the corners, with where.
        let r = s.execute("stitch.check", &json!({"thresholds": {"maxDensity": 0.5, "maxTrims": 0}})).unwrap();
        let dense = r["findings"].as_array().unwrap().iter().find(|f| f["code"] == "density").expect("density");
        assert!(dense["where"]["x"].is_number() && dense["count"].as_u64().unwrap() > 0, "{dense}");
        assert!(codes(&r).contains(&"trims".to_string()));
        for bad in [json!({"thresholds": {"shortMm": 9}}), json!({"thresholds": {"nope": 1}}), json!({"thresholds": 3}), json!({"machine": "x"})] {
            assert!(s.execute("stitch.check", &bad).is_err(), "{bad}");
        }
        let mut empty = Session::new();
        empty.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let r = empty.execute("stitch.check", &json!({})).unwrap();
        assert_eq!(codes(&r), ["empty"]);
        assert_eq!(r["errors"], 1);
    }

    #[test]
    fn durations_read_as_hours_minutes_and_seconds() {
        assert_eq!(super::duration(40.4), "40 s");
        assert_eq!(super::duration(725.0), "12 min 05 s");
        assert_eq!(super::duration(3720.0), "1 h 02 min");
    }
}
