//! Embroidery stitch files (DST and EXP): the export options `document.export` and `stitch.info`
//! read, the stitch plan an export writes, and a stitch file opened as a document (`document.open`
//! with its `stitch` options).

use serde_json::Value;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_embroidery::{Output, ReadOptions, Start, WriteOptions, dst, exp};
use vectorcraft_stitch::{
    FromDocOptions, MachineProfile, Mode, PrepareOptions, StitchPlan, UNITS_PER_MM, from_document, prepare, profile, to_document,
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

/// The stitch options `document.formats` lists for DST and EXP. Lengths are in mm.
pub(crate) const OPTIONS: &[FormatOption] = &[
    FormatOption {
        name: "machine",
        ty: "string",
        default: "\"commercial15\"",
        description: "the machine's limits: commercial15 (15 needles, a 500 x 350 mm field) or generic (no needle or field limits)",
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
        description: "the longest stitch in mm, 1–12.1 (12.7 for EXP, its default too); longer ones are split by jumps",
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
    FormatOption { name: "start", ty: "string", default: "\"center\"", description: "the file's origin: center (of the stitches) or firstStitch" },
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
];

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

/// The largest move per axis a format holds, in plan units: DST 121, EXP 127.
fn max_move(cmd: &str, format: &str) -> Result<i32> {
    match format {
        "dst" => Ok(121),
        "exp" => Ok(127),
        other => Err(bad(cmd, format!("format `{other}`: a stitch file is dst or exp"))),
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

/// The stitch options in `p` for `format` (`dst` or `exp`). `mode` and `locks` stay `None` when
/// not given: [`planned`] decides them per layer (see [`FromDocOptions::mode`]).
fn options(cmd: &str, p: &Value, format: &str) -> Result<Options> {
    let limit = max_move(cmd, format)?;
    let max_mm = f64::from(limit) / UNITS_PER_MM;
    let machine = one_of(cmd, p, "machine", "commercial15", &["generic", "commercial15"])?;
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
    let mut profile = profile(machine).ok_or_else(|| bad(cmd, format!("machine `{machine}`: generic or commercial15")))?;
    profile.max_stitch = units(max_stitch).min(limit);
    // EXP's longer moves (127 units) serve its jumps too.
    profile.max_jump = if format == "exp" { limit } else { profile.max_jump.min(limit) };
    profile.min_stitch = units(min_stitch);
    profile.jumps_per_trim = jumps_per_trim;
    Ok(Options {
        from: FromDocOptions { mode, stitch_length: units(stitch_length), min_stitch: units(min_stitch), only: None },
        profile,
        locks,
        travel_trims,
        return_to_start,
        write: WriteOptions {
            start,
            return_to_start: return_to_start.unwrap_or(true),
            jumps_per_trim,
            trim_before_color_change: switch_or(p, "trimBeforeColorChange", false, cmd)?,
        },
        name,
    })
}

/// The plan for `doc` with options `o`, named, prepared for the machine, and its warnings. When
/// `locks` was not given, runs get lock stitches unless every run came from an opened stitch file
/// (which has its own), with a warning when the plan mixes the two. `travelTrims` follows the same
/// rule: art on an opened file's layer keeps the trims the file had and gets none added.
fn planned(cmd: &str, doc: &Document, o: &Options, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Vec<String>, bool)> {
    let from = FromDocOptions { only: selection, ..o.from.clone() };
    let mut made = from_document(doc, &from).map_err(|e| bad(cmd, e))?;
    let title = std::path::Path::new(&doc.title).file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.trim().is_empty());
    made.plan.name = o.name.clone().or(title).unwrap_or_else(|| "design".to_string());
    let locks = o.locks.unwrap_or(made.unmarked_runs > 0);
    if o.locks.is_none() && made.marked_runs > 0 && made.unmarked_runs > 0 {
        made.warnings.push(MIXED_LOCKS.to_string());
    }
    let travel_trims = o.travel_trims.unwrap_or(made.unmarked_runs > 0);
    let prepared = prepare(&made.plan, &o.profile, &PrepareOptions { locks, travel_trims }).map_err(|e| bad(cmd, e))?;
    let mut warnings = made.warnings;
    warnings.extend(prepared.warnings);
    // Everything stitched came from an opened stitch file.
    Ok((prepared.plan, warnings, made.unmarked_runs == 0))
}

/// What an export of `doc` as `format` (`dst` or `exp`) with options `p` writes: the stitch plan
/// (the stroked paths of `doc`, only `selection` and what it holds when given, prepared for the
/// machine), the file, and the warnings of both. Errors name `cmd`.
pub(crate) fn written(cmd: &str, doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Output)> {
    let o = options(cmd, p, format)?;
    let (plan, mut warnings, opened) = planned(cmd, doc, &o, selection)?;
    // Not given: an opened file did not return to its start, so its re-export adds no such travel.
    let write = WriteOptions { return_to_start: o.return_to_start.unwrap_or(!opened), ..o.write.clone() };
    let mut out = match format {
        "exp" => exp::write(&plan, &write),
        _ => dst::write(&plan, &write),
    }
    .map_err(|e| bad(cmd, e))?;
    warnings.append(&mut out.warnings);
    out.warnings = warnings;
    Ok((plan, out))
}

/// The stitch plan an export of `doc` as `format` with options `p` writes, and its warnings,
/// without writing the file (the writer's own warnings are left out). As [`written`] otherwise.
pub(crate) fn plan(cmd: &str, doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Vec<String>)> {
    let o = options(cmd, p, format)?;
    planned(cmd, doc, &o, selection)
}

/// `doc` as a DST or EXP file. `selection`: only those objects; an export's `selectedOnly` has
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

/// A DST or EXP file as a document: one layer of stroked paths, a group per thread colour, and the
/// reader's warnings. `p`: the `stitch` options.
pub(super) fn import(bytes: &[u8], format: &str, p: Option<&Value>) -> Result<(Document, Vec<String>)> {
    let o = read_options("document.open", p)?;
    let input = match format {
        "dst" => dst::read(bytes, &o),
        "exp" => exp::read(bytes, &o),
        other => return Err(err(format!("{other} is not a stitch file format"))),
    }
    .map_err(err)?;
    // A hostile file can hold more steps, or more colour changes, than a document should take.
    input.plan.check().map_err(err)?;
    if input.plan.blocks.len() > MAX_BLOCKS {
        return Err(err("the file has too many colour changes"));
    }
    Ok((to_document(&input.plan), input.warnings))
}
