//! Embroidery thread charts and needles: `stitch.threads.list`, `stitch.threads.import`,
//! `stitch.threads.use`, `stitch.threads.match` and `stitch.needles`.
//!
//! A chart is a list of named threads with brand and catalogue code ([`ThreadChart`]). VectorCraft
//! ships only the small original `VectorCraft Starter` set; brand charts are imported from CSV or
//! JSON files and kept like the other user libraries, in the `Threads` folder next to the
//! preferences ([`Libraries`]). The active chart (`stitch.threads.use`) is kept for the session.

use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};
use vectorcraft_stitch::{MAX_CHART_BYTES, NeedleUse, ThreadChart, ThreadMatch, assign_needles, hex, profile, starter_chart};

use super::fileio::{read_file_head, stitchfile};
use super::swatchlib::{LibraryFile, LibraryInfo, stem};
use super::{CommandSpec, Result, Session, always, bad, cmd, has_doc, switch_or, text_param};

const LIST: &str = "stitch.threads.list";
const IMPORT: &str = "stitch.threads.import";
const USE: &str = "stitch.threads.use";
const MATCH: &str = "stitch.threads.match";
const NEEDLES: &str = "stitch.needles";
/// The id of the built-in starter chart.
const STARTER_ID: &str = "builtin/starter";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "stitch.threads.list",
            "Thread Charts",
            [],
            None,
            "{chart?: id or name (also lists its threads)} → {charts: [{id, name, category: \"builtIn\" (the VectorCraft Starter set: 24 original colours, no maker's range)|\"user\" (the files of the Threads folder next to the preferences, rescanned now)|\"loaded\" (stitch.threads.import without user), count, path?, active}], active: id|null, userFolder, threads?: [{brand, code, name, hex, weight?}]}",
            always,
            list
        ),
        cmd!(
            "stitch.threads.import",
            "Import Thread Chart",
            [],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's), user?: false (also keep it in the Threads folder, so it is listed in every session)} read a thread chart, CSV (a header row with the columns code and hex, and optionally brand, name and weight, in any order; quote fields holding commas with \"; hex is #rrggbb) or JSON ({\"format\": \"vectorcraft-threads\", \"name\"?, \"threads\": [{brand?, code, name?, hex, weight?}]}); at most 20000 threads, 4 MB, brand 64, code 32, name 96 characters; a code listed twice for a brand, a bad colour, a field too long or a file that is not UTF-8 text is an error naming the line → {chart: id, name, count, category, path?, copied: whether a file was written to the Threads folder}",
            always,
            import
        ),
        cmd!(
            "stitch.threads.use",
            "Use Thread Chart",
            [],
            None,
            "{name?: chart id or name (any case; none or not given: no chart)} make a chart the active one: stitch.info and stitch.needles then name the nearest thread of it for each colour (matching is by CIE76 delta E in CIE L*a*b*) → {chart: id|null, name?, count?}; kept for the session",
            always,
            use_chart
        ),
        cmd!(
            query "stitch.threads.match",
            "Match Threads",
            [],
            None,
            "{chart?: id or name (default: the active chart), …the plan options of stitch.info (format, machine, mode, …, selectedOnly)} the nearest thread of the chart to each thread colour of the stitch plan that export would write → {chart: id, name, blocks: [{block: index in sewing order, rgb: \"#rrggbb\", brand, code, name, thread: \"#rrggbb\" of the chart thread, deltaE}]}; a document with no stroked paths is an error",
            has_doc,
            match_cmd
        ),
        cmd!(
            query "stitch.needles",
            "Needle Assignment",
            [],
            None,
            "{machine?: commercial15 (default) | bai_mirror | generic (no needle limit), chart?: id or name (default: the active chart; names the nearest thread for each needle), …the plan options of stitch.info} the needle each thread colour of the stitch plan is sewn with: the first colour sewn on needle 1, the next new colour on needle 2 and so on; a colour sewn again keeps its needle → {machine, needleCount: the machine's needles|null, needles: [{needle: from 1 (null: none left, a thread swap during the job), rgb: \"#rrggbb\", stitches, blocks: [indexes in sewing order], chart?: {brand, code, name, thread, deltaE}}], warnings (the colours past the needle count)}. Informational: the DSB writer still changes needles in order and does not read this assignment; a document with no stroked paths is an error",
            has_doc,
            needles_cmd
        ),
    ]
}

impl LibraryFile for ThreadChart {
    const EXTS: &'static [&'static str] = &["csv", "json"];
    fn read(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        ThreadChart::parse(bytes, stem)
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// User Defined and loaded thread charts.
pub type Libraries = super::swatchlib::Libraries<ThreadChart>;

fn starter() -> Arc<ThreadChart> {
    static STARTER: OnceLock<Arc<ThreadChart>> = OnceLock::new();
    STARTER.get_or_init(|| Arc::new(starter_chart())).clone()
}

fn starter_info() -> LibraryInfo {
    LibraryInfo { id: STARTER_ID.into(), name: vectorcraft_stitch::STARTER_NAME.into(), category: "builtIn" }
}

/// Every chart: the starter set, then the Threads folder's and the loaded ones.
fn charts(s: &Session) -> Vec<LibraryInfo> {
    std::iter::once(starter_info()).chain(s.thread_charts.infos().cloned()).collect()
}

/// Chart `key` (an id, or a name in any case) with its info.
pub(crate) fn chart(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<ThreadChart>)> {
    if key == STARTER_ID {
        return Some((starter_info(), starter()));
    }
    if let Some(found) = s.thread_charts.get(key) {
        return Some(found);
    }
    let id = charts(s).into_iter().find(|l| l.name.eq_ignore_ascii_case(key))?.id;
    if id == STARTER_ID { Some((starter_info(), starter())) } else { s.thread_charts.get(&id) }
}

/// The chart `stitch.threads.use` made active, if it is still there.
pub(crate) fn active(s: &Session) -> Option<(LibraryInfo, Arc<ThreadChart>)> {
    chart(s, s.active_thread_chart.as_deref()?)
}

/// The chart `chart` names in `p`, else the active one.
fn chart_or_active(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<ThreadChart>)> {
    match text_param(p, "chart", cmd)? {
        Some(key) => chart(s, key).ok_or_else(|| bad(cmd, format!("no thread chart `{key}` (see stitch.threads.list)"))),
        None => active(s).ok_or_else(|| bad(cmd, "no thread chart is active: pass `chart`, or choose one with stitch.threads.use")),
    }
}

/// The JSON for a thread of `chart` matched to a colour.
pub(crate) fn match_json(chart: &ThreadChart, m: ThreadMatch) -> Value {
    let Some(t) = chart.threads.get(m.index) else { return Value::Null };
    json!({
        "brand": t.brand,
        "code": t.code,
        "name": t.name,
        "thread": hex(t.rgb),
        "deltaE": (m.delta_e * 100.0).round() / 100.0,
    })
}

fn thread_json(t: &vectorcraft_stitch::ThreadSpec) -> Value {
    let mut v = json!({"brand": t.brand, "code": t.code, "name": t.name, "hex": hex(t.rgb)});
    if let Some(w) = &t.weight {
        v["weight"] = json!(w);
    }
    v
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    s.thread_charts.rescan();
    let active_id = active(s).map(|(i, _)| i.id);
    let rows: Vec<Value> = charts(s)
        .into_iter()
        .map(|l| {
            let count = chart(s, &l.id).map_or(0, |(_, c)| c.threads.len());
            json!({"id": l.id, "name": l.name, "category": l.category, "count": count, "path": s.thread_charts.path(&l.id), "active": active_id.as_deref() == Some(l.id.as_str())})
        })
        .collect();
    let mut out = json!({"charts": rows, "active": active_id, "userFolder": s.thread_charts.user_dir()});
    if let Some(key) = text_param(p, "chart", LIST)? {
        let (_, c) = chart(s, key).ok_or_else(|| bad(LIST, format!("no thread chart `{key}`")))?;
        out["threads"] = Value::Array(c.threads.iter().map(thread_json).collect());
    }
    Ok(out)
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let user = switch_or(p, "user", false, IMPORT)?;
    if user && s.thread_charts.user_dir().is_none() {
        return Err(bad(IMPORT, "no Threads folder here (import without user, or with a path to read from)"));
    }
    // A file past the cap is refused before it is read in full.
    if let Some(path) = text_param(p, "path", IMPORT)?
        && read_file_head(path, MAX_CHART_BYTES + 1)?.len() > MAX_CHART_BYTES
    {
        return Err(bad(IMPORT, format!("the chart is larger than {} MB", MAX_CHART_BYTES >> 20)));
    }
    let (mut info, mut lib) = s.thread_charts.load(p, IMPORT, |bytes, file| ThreadChart::parse(bytes, stem(file)).map_err(|e| bad(IMPORT, e)))?;
    let mut copied = false;
    if user && info.category == "loaded" {
        let json = |c: &ThreadChart| Ok(c.to_json().into_bytes());
        (info, lib, copied) = s.thread_charts.copy_to_user(&info.id, "json", json, IMPORT)?;
    } else if user && info.category != "user" {
        return Err(bad(IMPORT, "the chart could not be kept in the Threads folder"));
    }
    Ok(
        json!({"chart": info.id, "name": info.name, "count": lib.threads.len(), "category": info.category, "path": s.thread_charts.path(&info.id), "copied": copied}),
    )
}

fn use_chart(s: &mut Session, p: &Value) -> Result<Value> {
    let key = text_param(p, "name", USE)?.filter(|k| !k.eq_ignore_ascii_case("none"));
    let Some(key) = key else {
        s.active_thread_chart = None;
        return Ok(json!({"chart": null}));
    };
    s.thread_charts.rescan();
    let (info, c) = chart(s, key).ok_or_else(|| bad(USE, format!("no thread chart `{key}` (see stitch.threads.list)")))?;
    s.active_thread_chart = Some(info.id.clone());
    Ok(json!({"chart": info.id, "name": info.name, "count": c.threads.len()}))
}

/// The stitch plan an export with the stitch options in `p` would write.
fn plan_of(s: &Session, p: &Value, cmd: &str) -> Result<vectorcraft_stitch::StitchPlan> {
    let format = text_param(p, "format", cmd)?.unwrap_or("dst");
    let selection = super::stitch::selected(s, p, cmd)?;
    Ok(stitchfile::plan(cmd, &s.doc()?.doc, p, format, selection)?.0)
}

fn match_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, c) = chart_or_active(s, p, MATCH)?;
    let plan = plan_of(s, p, MATCH)?;
    let blocks: Vec<Value> = plan
        .blocks
        .iter()
        .zip(c.match_plan(&plan))
        .enumerate()
        .map(|(i, (b, m))| {
            let mut v = m.map_or(Value::Null, |m| match_json(&c, m));
            if let Some(o) = v.as_object_mut() {
                o.insert("block".into(), json!(i));
                o.insert("rgb".into(), json!(hex(b.thread.rgb)));
            }
            v
        })
        .collect();
    Ok(json!({"chart": info.id, "name": info.name, "blocks": blocks}))
}

fn needles_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let machine = text_param(p, "machine", NEEDLES)?.unwrap_or("commercial15");
    let prof = profile(machine).ok_or_else(|| bad(NEEDLES, format!("machine `{machine}`: generic, commercial15 or bai_mirror")))?;
    let chart = match text_param(p, "chart", NEEDLES)? {
        Some(_) => Some(chart_or_active(s, p, NEEDLES)?),
        None => active(s),
    };
    let plan = plan_of(s, p, NEEDLES)?;
    let n = assign_needles(&plan, prof.needles);
    let needles: Vec<Value> = n
        .uses
        .iter()
        .map(|u: &NeedleUse| {
            let mut v = json!({"needle": u.needle, "rgb": hex(u.rgb), "stitches": u.stitches, "blocks": u.blocks});
            if let Some((_, c)) = &chart {
                v["chart"] = c.nearest(u.rgb).map_or(Value::Null, |m| match_json(c, m));
            }
            v
        })
        .collect();
    Ok(json!({"machine": prof.id, "needleCount": prof.needles, "needles": needles, "warnings": n.warnings}))
}
