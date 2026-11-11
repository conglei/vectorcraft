//! Embroidery stitch files (DST and EXP): the export options `document.export` and `stitch.info`
//! read, the stitch plan an export writes, and a stitch file opened as a document (`document.open`
//! with its `stitch` options).

use serde_json::Value;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_embroidery::{ReadOptions, Start, WriteOptions, dst, exp};
use vectorcraft_stitch::{
    FromDocOptions, MachineProfile, Mode, PrepareOptions, StitchPlan, UNITS_PER_MM, from_document, prepare, profile, to_document,
};

use super::super::{Result, bad, number_or, switch_or, text_param};
use super::{Encoded, FormatOption, err};

const C: &str = "document.export";

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
        description: "run (running stitch along each stroked path) or manual (one needle point per anchor)",
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
    FormatOption { name: "locks", ty: "boolean", default: "true", description: "lock stitches at the start and end of each run" },
    FormatOption {
        name: "jumpsPerTrim",
        ty: "integer",
        default: "3",
        description: "how many jumps in a row the machine reads as a trim, 2–8; travel that long gets a trim first",
    },
    FormatOption { name: "trimBeforeColorChange", ty: "boolean", default: "false", description: "a trim before each colour change" },
    FormatOption { name: "start", ty: "string", default: "\"center\"", description: "the file's origin: center (of the stitches) or firstStitch" },
    FormatOption { name: "returnToStart", ty: "boolean", default: "true", description: "the file ends with travel back to its origin" },
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
    prepare: PrepareOptions,
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

/// A length in mm in plan units. `mm` is finite and at most 12, so it fits.
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

/// The stitch options in `p` for `format` (`dst` or `exp`).
fn options(cmd: &str, p: &Value, format: &str) -> Result<Options> {
    let limit = max_move(cmd, format)?;
    let max_mm = f64::from(limit) / UNITS_PER_MM;
    let machine = one_of(cmd, p, "machine", "commercial15", &["generic", "commercial15"])?;
    let mode = match one_of(cmd, p, "mode", "run", &["run", "manual"])? {
        "manual" => Mode::Manual,
        _ => Mode::Run,
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
        prepare: PrepareOptions { locks: switch_or(p, "locks", true, cmd)? },
        write: WriteOptions {
            start,
            return_to_start: switch_or(p, "returnToStart", true, cmd)?,
            jumps_per_trim,
            trim_before_color_change: switch_or(p, "trimBeforeColorChange", false, cmd)?,
        },
        name,
    })
}

/// The plan for `doc` with options `o`, named, prepared for the machine, and its warnings.
fn planned(cmd: &str, doc: &Document, o: &Options, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Vec<String>)> {
    let from = FromDocOptions { only: selection, ..o.from.clone() };
    let mut made = from_document(doc, &from).map_err(|e| bad(cmd, e))?;
    let title = std::path::Path::new(&doc.title).file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.trim().is_empty());
    made.plan.name = o.name.clone().or(title).unwrap_or_else(|| "design".to_string());
    let prepared = prepare(&made.plan, &o.profile, &o.prepare).map_err(|e| bad(cmd, e))?;
    let mut warnings = made.warnings;
    warnings.extend(prepared.warnings);
    Ok((prepared.plan, warnings))
}

/// The stitch plan an export of `doc` as `format` (`dst` or `exp`) with options `p` writes, and its
/// warnings: the stroked paths of `doc` (only `selection` and what it holds, when given) as
/// stitches, prepared for the machine. Errors name `cmd`.
pub(crate) fn plan(cmd: &str, doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<(StitchPlan, Vec<String>)> {
    planned(cmd, doc, &options(cmd, p, format)?, selection)
}

/// `doc` as a DST or EXP file. `selection`: only those objects; an export's `selectedOnly` has
/// already left the rest out of `doc`, so it passes `None`.
pub(super) fn encode(doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<Encoded> {
    let o = options(C, p, format)?;
    let (plan, mut warnings) = planned(C, doc, &o, selection)?;
    let out = match format {
        "exp" => exp::write(&plan, &o.write),
        _ => dst::write(&plan, &o.write),
    }
    .map_err(|e| bad(C, e))?;
    warnings.extend(out.warnings);
    Ok(Encoded { warnings, ..Encoded::one(out.bytes) })
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
