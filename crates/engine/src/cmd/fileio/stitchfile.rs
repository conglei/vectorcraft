//! Embroidery stitch files (DST, DSB, EXP, U01, TBF, PES and PEC): the export options `document.export` and `stitch.info`
//! read, the stitch plan an export writes, and a stitch file opened as a document (`document.open`
//! with its `stitch` options).

use serde_json::Value;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_embroidery::{MAX_NEEDLE, Output, ReadOptions, Start, WriteOptions, read_format, write_format};
use vectorcraft_stitch::{
    FillOptions, FillParams, FillType, FromDocOptions, MachineProfile, Mode, PrepareOptions, Pt, StitchPlan, StrokeOptions, StrokeType, UNITS_PER_MM,
    assign_needles, from_document, prepare_tagged, profile, to_document,
};

use super::super::{Result, bad, number_or, switch_or, text_param};
use super::{Encoded, FormatOption, err};

const C: &str = "document.export";

/// The warning for lock stitches added by default to a plan that mixes an opened stitch file's
/// runs with other art.
const MIXED_LOCKS: &str =
    "lock stitches were added to every run, including those of the opened stitch file: pass locks: false to keep that file's runs unchanged";

/// The most colour changes an opened file may have: each becomes a group in the document.
const MAX_BLOCKS: usize = 10_000;

/// The stitch options `document.formats` lists for the stitch file formats. Lengths are in mm.
pub(crate) const OPTIONS: &[FormatOption] = &[
    FormatOption {
        name: "machine",
        ty: "string",
        default: "\"commercial15\"",
        description: "the machine's limits: commercial15 (15 needles, a 500 x 350 mm field), bai_mirror (a commercial single-head 15-needle machine, 350 x 500 mm; the same limits, and a warning for a format its panel does not read) or generic (no needle or field limits)",
    },
    FormatOption {
        name: "mode",
        ty: "string",
        default: "\"run\"",
        description: "run (running stitch along each stroked path) or manual (one needle point per anchor), for all art; not given: art on the layer an opened stitch file made is taken anchor by anchor, so its needle points export unchanged, and all other art is run",
    },
    FormatOption { name: "stitchLength", ty: "number", default: "2.5", description: "running stitch length in mm, 0.5–12" },
    FormatOption {
        name: "minStitch",
        ty: "number",
        default: "0.3",
        description: "stitches shorter than this (mm, 0–2) are dropped; runs that start this close to the last stitch are joined",
    },
    FormatOption {
        name: "maxStitch",
        ty: "number",
        default: "12.1",
        description: "the longest stitch in mm, 1–12.1 (12.7 for EXP, DSB, U01 and TBF, their default too); longer ones are split into equal pieces: jumps and a final stitch",
    },
    FormatOption {
        name: "locks",
        ty: "boolean",
        default: "true",
        description: "lock stitches at the start and end of each run; not given: off when everything stitched is on the layer an opened stitch file made (it has its own), on otherwise, with a warning when that file's runs get them too",
    },
    FormatOption {
        name: "travelTrims",
        ty: "boolean",
        default: "true",
        description: "a trim before travel long enough that the machine would read it as one (see jumpsPerTrim); trims the plan already has stay either way; not given: off when everything stitched is on the layer an opened stitch file made (its own trims are kept, none are added), on otherwise",
    },
    FormatOption {
        name: "jumpsPerTrim",
        ty: "integer",
        default: "3",
        description: "how many jumps in a row the machine reads as a trim, 2–8; travel that long gets a trim first",
    },
    FormatOption { name: "trimBeforeColorChange", ty: "boolean", default: "false", description: "a trim before each colour change" },
    FormatOption {
        name: "start",
        ty: "string",
        default: "\"center\"",
        description: "the file's origin: center (of the stitches) or firstStitch; a design start set with stitch.designStartEnd.set is the origin instead",
    },
    FormatOption {
        name: "returnToStart",
        ty: "boolean",
        default: "true",
        description: "the file ends with travel back to its origin; not given: off when everything stitched is on the layer an opened stitch file made (re-exporting it adds no travel the file did not have), on otherwise",
    },
    FormatOption {
        name: "name",
        ty: "string",
        default: "null",
        description: "the design name in the file (default: the document's title without its extension)",
    },
    FormatOption { name: "selectedOnly", ty: "boolean", default: "false", description: "only the selected objects" },
    FormatOption { name: "fill", ty: "object", default: "null", description: FILL_DOC },
    FormatOption { name: "stroke", ty: "object", default: "null", description: STROKE_DOC },
];

/// What the `fill` option takes.
pub(crate) const FILL_DOC: &str = "how filled paths are sewn: {type?: tatami (default; rows of running stitch, holes kept by the path's fill rule, sewn before the path's outline) | none (fills are not sewn), angle?: 0 (row direction in degrees, counter-clockwise from horizontal, -360–360), spacing?: 0.4 (mm between rows, 0.2–5), stitchLength?: 3 (mm along a row, 1–8), stagger?: 0.25 (how far each row's penetrations move along the row, as a fraction of the stitch length, 0–1)}; not used on the layer an opened stitch file made";

/// What the `stroke` option takes.
pub(crate) const STROKE_DOC: &str = "how stroked paths are sewn: {type?: run (default; running stitch, or anchor by anchor by mode) | satin (a satin column centred on the path, square ends, stitches longer than maxStitch split), spacing?: 0.4 (satin, mm peak to peak, 0.2–5), width?: (satin, mm rail to rail, 0.5–20; default the stroke weight, held to 0.5–20 with a warning)}; satin is not used on the layer an opened stitch file made";

const FILL_KEYS: &[&str] = &["type", "angle", "spacing", "stitchLength", "stagger"];
const STROKE_KEYS: &[&str] = &["type", "spacing", "width"];

/// The object `key` of `p` (`None` when not given), with only `keys`.
fn sub_object<'a>(cmd: &str, p: &'a Value, key: &str, keys: &[&str]) -> Result<Option<&'a Value>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let Some(o) = v.as_object() else { return Err(bad(cmd, format!("{key} must be an object: {{{}}}", keys.join(", ")))) };
    if let Some(k) = o.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(bad(cmd, format!("unknown parameter `{key}.{k}`; {key} takes: {}", keys.join(", "))));
    }
    Ok(Some(v))
}

/// The number `key` of the `of` object `p` (default `default`), finite and within `lo..=hi`.
fn nested(cmd: &str, p: &Value, of: &str, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let unit = match key {
        "angle" => " degrees",
        "stagger" => "",
        _ => " mm",
    };
    let v = match p.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => v.as_f64().ok_or_else(|| bad(cmd, format!("{of}.{key} must be a number, not {v}")))?,
    };
    if !(v.is_finite() && (lo..=hi).contains(&v)) {
        return Err(bad(cmd, format!("{of}.{key} must be {lo}–{hi}{unit}, not {v}")));
    }
    Ok(v)
}

/// The string `key` of the `of` object `p` (default `default`), one of `allowed`.
fn nested_choice<'a>(cmd: &str, p: &'a Value, of: &str, key: &str, default: &'a str, allowed: &[&str]) -> Result<&'a str> {
    let v = match p.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => v.as_str().ok_or_else(|| bad(cmd, format!("{of}.{key} must be a string, not {v}")))?,
    };
    if !allowed.contains(&v) {
        return Err(bad(cmd, format!("{of}.{key} `{v}`: {}", allowed.join(" or "))));
    }
    Ok(v)
}

/// The `fill` option.
fn fill_options(cmd: &str, p: &Value) -> Result<FillOptions> {
    let Some(f) = sub_object(cmd, p, "fill", FILL_KEYS)? else { return Ok(FillOptions::default()) };
    let d = FillParams::default();
    let kind = match nested_choice(cmd, f, "fill", "type", "tatami", &["tatami", "none"])? {
        "none" => FillType::None,
        _ => FillType::Tatami,
    };
    let params = FillParams {
        angle: nested(cmd, f, "fill", "angle", d.angle, -360.0, 360.0)?,
        spacing: nested(cmd, f, "fill", "spacing", d.spacing, 0.2, 5.0)?,
        stitch_length: nested(cmd, f, "fill", "stitchLength", d.stitch_length, 1.0, 8.0)?,
        stagger: nested(cmd, f, "fill", "stagger", d.stagger, 0.0, 1.0)?,
        max_stitch: d.max_stitch,
    };
    Ok(FillOptions { kind, params })
}

/// The `stroke` option.
fn stroke_options(cmd: &str, p: &Value) -> Result<StrokeOptions> {
    let Some(s) = sub_object(cmd, p, "stroke", STROKE_KEYS)? else { return Ok(StrokeOptions::default()) };
    let d = StrokeOptions::default();
    let kind = match nested_choice(cmd, s, "stroke", "type", "run", &["run", "satin"])? {
        "satin" => StrokeType::Satin,
        _ => StrokeType::Run,
    };
    let width = match s.get("width") {
        None | Some(Value::Null) => None,
        Some(_) => Some(nested(cmd, s, "stroke", "width", 0.0, 0.5, 20.0)?),
    };
    Ok(StrokeOptions { kind, spacing: nested(cmd, s, "stroke", "spacing", d.spacing, 0.2, 5.0)?, width })
}

/// The stitch options of an export, checked.
struct Options {
    from: FromDocOptions,
    profile: MachineProfile,
    /// Lock stitches, if given.
    locks: Option<bool>,
    /// Travel trims, if given.
    travel_trims: Option<bool>,
    /// Whether the file returns to its start, if given.
    return_to_start: Option<bool>,
    write: WriteOptions,
    name: Option<String>,
}

/// The largest move per axis a format holds, in plan units: DST, PES and PEC 121, EXP, DSB, U01
/// and TBF 127 (DSB and U01 records hold 255, PES records 2047, but the machines read 12.7 mm, and
/// Brother's 12.1 mm, as the longest stitch).
fn max_move(cmd: &str, format: &str) -> Result<i32> {
    match format {
        "dst" | "pes" | "pec" => Ok(121),
        "exp" | "dsb" | "u01" | "tbf" => Ok(127),
        other => Err(bad(cmd, format!("format `{other}`: a stitch file is dst, dsb, exp, u01, tbf, pes or pec"))),
    }
}

/// The number `key` in `p` (default `default`), finite and within `lo..=hi`.
fn length(cmd: &str, p: &Value, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    let v = number_or(p, key, default, cmd)?;
    if !(v.is_finite() && (lo..=hi).contains(&v)) {
        return Err(bad(cmd, format!("{key} must be {lo}–{hi} mm, not {v}")));
    }
    Ok(v)
}

/// A length in mm in plan units. `mm` is finite and at most 12.7, so it fits.
fn units(mm: f64) -> i32 {
    (mm * UNITS_PER_MM).round() as i32
}

/// The string `key` in `p` (default `default`), one of `allowed`.
fn one_of<'a>(cmd: &str, p: &'a Value, key: &str, default: &'a str, allowed: &[&str]) -> Result<&'a str> {
    let v = text_param(p, key, cmd)?.unwrap_or(default);
    if !allowed.contains(&v) {
        return Err(bad(cmd, format!("{key} `{v}`: {}", allowed.join(" or "))));
    }
    Ok(v)
}

/// How many jumps in a row are a trim: `jumpsPerTrim` in `p`, a whole number 2–8.
fn jumps_per_trim(cmd: &str, p: &Value, key: &str) -> Result<u8> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(3),
        Some(v) => v
            .as_u64()
            .filter(|n| (2..=8).contains(n))
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| bad(cmd, format!("{key} must be a whole number 2–8, not {v}"))),
    }
}

/// The stitch options in `p` for `format` (one of `vectorcraft_embroidery::FORMATS`). `mode` and `locks` stay `None` when
/// not given: [`planned`] decides them per layer (see [`FromDocOptions::mode`]).
fn options(cmd: &str, p: &Value, format: &str) -> Result<Options> {
    let limit = max_move(cmd, format)?;
    let max_mm = f64::from(limit) / UNITS_PER_MM;
    let machine = one_of(cmd, p, "machine", "commercial15", &["generic", "commercial15", "bai_mirror"])?;
    let mode = match text_param(p, "mode", cmd)? {
        None => None,
        Some(_) => match one_of(cmd, p, "mode", "run", &["run", "manual"])? {
            "manual" => Some(Mode::Manual),
            _ => Some(Mode::Run),
        },
    };
    let locks = match p.get("locks") {
        None | Some(Value::Null) => None,
        Some(_) => Some(switch_or(p, "locks", true, cmd)?),
    };
    let travel_trims = match p.get("travelTrims") {
        None | Some(Value::Null) => None,
        Some(_) => Some(switch_or(p, "travelTrims", true, cmd)?),
    };
    let return_to_start = match p.get("returnToStart") {
        None | Some(Value::Null) => None,
        Some(_) => Some(switch_or(p, "returnToStart", true, cmd)?),
    };
    let stitch_length = length(cmd, p, "stitchLength", 2.5, 0.5, 12.0)?;
    let min_stitch = length(cmd, p, "minStitch", 0.3, 0.0, 2.0)?;
    let max_stitch = length(cmd, p, "maxStitch", max_mm, 1.0, max_mm)?;
    let jumps_per_trim = jumps_per_trim(cmd, p, "jumpsPerTrim")?;
    let start = match one_of(cmd, p, "start", "center", &["center", "firstStitch"])? {
        "firstStitch" => Start::FirstStitch,
        _ => Start::Center,
    };
    let name = text_param(p, "name", cmd)?.map(str::to_string);
    let mut profile = profile(machine).ok_or_else(|| bad(cmd, format!("machine `{machine}`: generic, commercial15 or bai_mirror")))?;
    profile.max_stitch = units(max_stitch).min(limit);
    // The longer moves of EXP, DSB, U01 and TBF (127 units) serve their jumps too.
    profile.max_jump = if limit > 121 { limit } else { profile.max_jump.min(limit) };
    profile.min_stitch = units(min_stitch);
    profile.jumps_per_trim = jumps_per_trim;
    Ok(Options {
        from: FromDocOptions {
            mode,
            stitch_length: units(stitch_length),
            min_stitch: units(min_stitch),
            only: None,
            max_stitch: profile.max_stitch,
            fill: fill_options(cmd, p)?,
            stroke: stroke_options(cmd, p)?,
        },
        profile,
        locks,
        travel_trims,
        return_to_start,
        write: WriteOptions {
            start,
            return_to_start: return_to_start.unwrap_or(true),
            jumps_per_trim,
            trim_before_color_change: switch_or(p, "trimBeforeColorChange", false, cmd)?,
            needles: Vec::new(),
        },
        name,
    })
}

/// The document-wide stitch settings the export options `p` give (as for a DST file): the
/// defaults an object's stitch properties fall back on (`stitch.props`). Errors name `cmd`.
pub(crate) fn from_doc_options(cmd: &str, p: &Value) -> Result<FromDocOptions> {
    options(cmd, p, "dst").map(|o| o.from)
}

/// Check the stitch options in `p` for `format` (one of `vectorcraft_embroidery::FORMATS`) without planning a file (dialogs
/// keep themselves open on an error).
pub fn check(p: &Value, format: &str) -> std::result::Result<(), String> {
    options(C, p, format).map(|_| ()).map_err(|e| e.to_string())
}

/// A plan prepared for the machine, and which object sewed each of its steps.
pub(crate) struct Made {
    pub plan: StitchPlan,
    pub warnings: Vec<String>,
    /// Everything stitched came from an opened stitch file.
    pub opened: bool,
    /// The objects in sewing order, and for each step of `plan` (`tags[block][step]`) the index
    /// into `owners` of the object it sews ([`vectorcraft_stitch::Planned::tags`]).
    pub owners: Vec<NodeId>,
    pub tags: Vec<Vec<u32>>,
}

/// The plan `stitch.info` counts (a DST export of `doc` with options `p`), with the object each
/// step sews: the one accounting the Stitch Sequence panel, the player and the export share.
pub(crate) fn accounted(cmd: &str, doc: &Document, p: &Value) -> Result<Made> {
    let o = options(cmd, p, "dst")?;
    planned(cmd, doc, &o, None)
}

/// The plan for `doc` with options `o`, named, prepared for the machine, and its warnings. When
/// `locks` was not given, runs get lock stitches unless every run came from an opened stitch file
/// (which has its own), with a warning when the plan mixes the two. `travelTrims` follows the same
/// rule: art on an opened file's layer keeps the trims the file had and gets none added.
fn planned(cmd: &str, doc: &Document, o: &Options, selection: Option<Vec<NodeId>>) -> Result<Made> {
    let from = FromDocOptions { only: selection, ..o.from.clone() };
    let mut made = from_document(doc, &from).map_err(|e| bad(cmd, e))?;
    let title = std::path::Path::new(&doc.title).file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.trim().is_empty());
    made.plan.name = o.name.clone().or(title).unwrap_or_else(|| "design".to_string());
    let locks = o.locks.unwrap_or(made.unmarked_runs > 0);
    if o.locks.is_none() && made.marked_runs > 0 && made.unmarked_runs > 0 {
        made.warnings.push(MIXED_LOCKS.to_string());
    }
    let travel_trims = o.travel_trims.unwrap_or(made.unmarked_runs > 0);
    let (prepared, tags) = prepare_tagged(&made.plan, &made.tags, &o.profile, &PrepareOptions { locks, travel_trims }).map_err(|e| bad(cmd, e))?;
    let mut warnings = made.warnings;
    warnings.extend(prepared.warnings);
    // Everything stitched came from an opened stitch file.
    Ok(Made { plan: prepared.plan, warnings, opened: made.unmarked_runs == 0, owners: made.owners, tags })
}

/// What an export of `doc` as `format` (one of `vectorcraft_embroidery::FORMATS`) with options `p` writes: the stitch plan
/// (the stroked paths of `doc`, only `selection` and what it holds when given, prepared for the
/// machine), the file, and the warnings of both. Errors name `cmd`.
pub(crate) fn written(cmd: &str, doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Output)> {
    let o = options(cmd, p, format)?;
    let Made { plan, mut warnings, opened, .. } = planned(cmd, doc, &o, selection)?;
    if !o.profile.formats.is_empty() && !o.profile.formats.contains(&format) {
        let list = o.profile.formats.iter().map(|f| f.to_uppercase()).collect::<Vec<_>>().join(" and ");
        warnings.push(format!("this machine's panel reads {list}, not {}", format.to_uppercase()));
    }
    // Not given: an opened file did not return to its start, so its re-export adds no such travel.
    let mut write = WriteOptions { return_to_start: o.return_to_start.unwrap_or(!opened), ..o.write.clone() };
    // The design start (Embroidery › Sequence) is the origin; the `start` option is the fallback.
    if let Some(p) = doc.stitch.start_point().and_then(|p| Pt::from_pt(p.x, p.y)) {
        write.start = Start::Point(p);
    }
    // U01 and TBF name a needle per block: the machine's assignment (`stitch.needles`), a colour
    // sewn again on its own needle; colours past the machine's needles take the next in turn.
    let max_needles = o.profile.needles.unwrap_or(MAX_NEEDLE).min(MAX_NEEDLE);
    let uses = assign_needles(&plan, Some(max_needles)).uses;
    write.needles = (0..plan.blocks.len())
        .map(|i| uses.iter().find(|u| u.blocks.contains(&i)).and_then(|u| u.needle).and_then(|n| u8::try_from(n).ok()).unwrap_or(0))
        .collect();
    let mut out = write_format(format, &plan, &write).map_err(|e| bad(cmd, e))?;
    warnings.append(&mut out.warnings);
    out.warnings = warnings;
    Ok((plan, out))
}

/// The stitch plan an export of `doc` as `format` with options `p` writes, and its warnings,
/// without writing the file (the writer's own warnings are left out). As [`written`] otherwise.
pub(crate) fn plan(cmd: &str, doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Vec<String>)> {
    let o = options(cmd, p, format)?;
    // Whether everything came from an opened file only decides the writer's return to the start.
    let Made { plan, warnings, .. } = planned(cmd, doc, &o, selection)?;
    Ok((plan, warnings))
}

/// `doc` as a stitch file. `selection`: only those objects; an export's `selectedOnly` has
/// already left the rest out of `doc`, so it passes `None`.
pub(super) fn encode(doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<Encoded> {
    let (_, out) = written(C, doc, p, format, selection)?;
    Ok(Encoded { warnings: out.warnings, ..Encoded::one(out.bytes) })
}

/// The stitch read options (`document.open`'s `stitch` object): `{jumpsPerTrim?: 2–8}`.
pub(super) fn read_options(cmd: &str, p: Option<&Value>) -> Result<ReadOptions> {
    let Some(p) = p.filter(|v| !v.is_null()) else { return Ok(ReadOptions::default()) };
    let Some(o) = p.as_object() else { return Err(bad(cmd, "stitch must be an object of stitch file options: {jumpsPerTrim?}")) };
    if let Some(key) = o.keys().find(|k| k.as_str() != "jumpsPerTrim") {
        return Err(bad(cmd, format!("unknown parameter `stitch.{key}`; stitch takes: jumpsPerTrim")));
    }
    Ok(ReadOptions { jumps_per_trim: jumps_per_trim(cmd, p, "jumpsPerTrim")? })
}

/// A stitch file as a document: one layer of stroked paths, a group per thread colour, and the
/// reader's warnings. `p`: the `stitch` options.
pub(super) fn import(bytes: &[u8], format: &str, p: Option<&Value>) -> Result<(Document, Vec<String>)> {
    let o = read_options("document.open", p)?;
    let input = read_format(format, bytes, &o).map_err(err)?;
    // A hostile file can hold more steps, or more colour changes, than a document should take.
    input.plan.check().map_err(err)?;
    if input.plan.blocks.len() > MAX_BLOCKS {
        return Err(err("the file has too many colour changes"));
    }
    Ok((to_document(&input.plan), input.warnings))
}
