//! Thread charts: lists of named threads with a brand and a catalogue code, the starter palette,
//! nearest-colour matching and needle assignment.
//!
//! A chart is imported from a CSV or JSON file (see [`ThreadChart::parse`]); VectorCraft ships no
//! brand chart, only [`starter_chart`], a small original set that is not any maker's range.
//! Matching measures distance in CIE L*a*b* (D65) as the plain Euclidean distance, CIE76 delta E:
//! it is cheap and good enough to pick the closest of a few hundred threads; CIEDE2000 would
//! separate near neighbours better and may replace it.

use crate::{Step, StitchPlan, Thread};

/// The most threads a chart holds.
pub const MAX_CHART_THREADS: usize = 20_000;
/// The largest chart file read, in bytes.
pub const MAX_CHART_BYTES: usize = 4 << 20;
/// The longest brand, in characters.
pub const MAX_BRAND: usize = 64;
/// The longest catalogue code, in characters.
pub const MAX_CODE: usize = 32;
/// The longest thread name, in characters.
pub const MAX_NAME: usize = 96;
/// The longest weight text (`40wt`), in characters.
pub const MAX_WEIGHT: usize = 16;
/// The longest chart name, in characters.
pub const MAX_CHART_NAME: usize = 96;
/// The `format` header of a JSON chart.
pub const JSON_FORMAT: &str = "vectorcraft-threads";

/// One thread in a chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadSpec {
    pub brand: String,
    /// The catalogue number.
    pub code: String,
    pub name: String,
    pub rgb: [u8; 3],
    /// The thread weight as the maker states it (`40wt`), if the chart gives one.
    pub weight: Option<String>,
}

impl ThreadSpec {
    /// The plan [`Thread`] for this spec.
    pub fn thread(&self) -> Thread {
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        Thread { rgb: self.rgb, name: some(&self.name), code: some(&self.code), brand: some(&self.brand) }
    }
}

/// A named list of threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadChart {
    pub name: String,
    pub threads: Vec<ThreadSpec>,
}

/// The nearest chart thread to a colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreadMatch {
    /// Index into [`ThreadChart::threads`].
    pub index: usize,
    /// CIE76 delta E between the two colours: 0 is the same colour, about 2.3 the least a person
    /// notices side by side.
    pub delta_e: f64,
}

/// CIE L*a*b* of an sRGB colour, D65 white.
pub fn lab(rgb: [u8; 3]) -> [f64; 3] {
    let lin = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(rgb[0]), lin(rgb[1]), lin(rgb[2]));
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
    let z = (0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f64| if t > 216.0 / 24_389.0 { t.cbrt() } else { (24_389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The CIE76 delta E between two sRGB colours.
pub fn delta_e(a: [u8; 3], b: [u8; 3]) -> f64 {
    distance(lab(a), lab(b))
}

impl ThreadChart {
    /// The thread nearest to `rgb`; the first of equals. `None` for an empty chart.
    pub fn nearest(&self, rgb: [u8; 3]) -> Option<ThreadMatch> {
        let target = lab(rgb);
        let mut best: Option<ThreadMatch> = None;
        for (index, t) in self.threads.iter().enumerate() {
            let d = distance(lab(t.rgb), target);
            if best.is_none_or(|b| d < b.delta_e) {
                best = Some(ThreadMatch { index, delta_e: d });
            }
        }
        best
    }

    /// The match for each block of `plan`, in order.
    pub fn match_plan(&self, plan: &StitchPlan) -> Vec<Option<ThreadMatch>> {
        plan.blocks.iter().map(|b| self.nearest(b.thread.rgb)).collect()
    }

    /// Reads a chart file: JSON when its first character is `{`, CSV otherwise. `name` names a chart
    /// that does not name itself (CSV, or JSON without a `name`).
    pub fn parse(bytes: &[u8], name: &str) -> Result<ThreadChart, String> {
        let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        match body.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{') => Self::parse_json(bytes, name),
            _ => Self::parse_csv(bytes, name),
        }
    }

    /// Reads a CSV chart: a header row with the columns `code` and `hex` and optionally `brand`,
    /// `name` and `weight` in any order, then one thread per row. Fields may be quoted with `"`
    /// (`""` inside); blank lines are skipped; `hex` is `#rrggbb` or `rrggbb`. A missing `brand`
    /// is the chart's name. Errors name the line.
    pub fn parse_csv(bytes: &[u8], name: &str) -> Result<ThreadChart, String> {
        let text = text_of(bytes)?;
        let name = chart_name(name)?;
        let mut rows = csv_rows(text)?.into_iter();
        let (head_line, head) = rows.next().ok_or("the chart is empty")?;
        let col = |key: &str| head.iter().position(|h| h.trim().eq_ignore_ascii_case(key));
        let code_col = col("code").ok_or_else(|| format!("line {head_line}: the header needs a `code` column"))?;
        let hex_col = col("hex").ok_or_else(|| format!("line {head_line}: the header needs a `hex` column"))?;
        let (brand_col, name_col, weight_col) = (col("brand"), col("name"), col("weight"));
        let mut threads = Vec::new();
        for (line, row) in rows {
            if row.len() != head.len() {
                return Err(format!("line {line}: {} fields, the header has {}", row.len(), head.len()));
            }
            let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|s| s.trim().to_string()).unwrap_or_default();
            let hex = get(Some(hex_col));
            let weight = get(weight_col);
            let spec = ThreadSpec {
                brand: get(brand_col),
                code: get(Some(code_col)),
                name: get(name_col),
                rgb: parse_hex(&hex).ok_or_else(|| format!("line {line}: `{}` is not a colour (#rrggbb)", shown(&hex)))?,
                weight: (!weight.is_empty()).then_some(weight),
            };
            push(&mut threads, spec, &name, &format!("line {line}"))?;
        }
        finish(name, threads)
    }

    /// Reads a JSON chart: `{"format": "vectorcraft-threads", "name"?: "…", "threads": [{"brand"?,
    /// "code", "name"?, "hex", "weight"?}, …]}`; `format` may be left out.
    pub fn parse_json(bytes: &[u8], name: &str) -> Result<ThreadChart, String> {
        let text = text_of(bytes)?;
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("not a thread chart: {e}"))?;
        let obj = v.as_object().ok_or("not a thread chart: expected an object")?;
        if let Some(f) = obj.get("format")
            && f.as_str() != Some(JSON_FORMAT)
        {
            return Err(format!("not a thread chart (format must be `{JSON_FORMAT}`)"));
        }
        let chart = match obj.get("name") {
            None | Some(serde_json::Value::Null) => name.to_string(),
            Some(n) => n.as_str().ok_or("the chart `name` must be text")?.to_string(),
        };
        let chart = chart_name(&chart)?;
        let list = obj.get("threads").and_then(|t| t.as_array()).ok_or("the chart needs a `threads` list")?;
        if list.len() > MAX_CHART_THREADS {
            return Err(too_many());
        }
        let mut threads = Vec::new();
        for (i, t) in list.iter().enumerate() {
            let at = format!("thread {}", i + 1);
            let o = t.as_object().ok_or_else(|| format!("{at}: expected an object"))?;
            let text = |key: &str| -> Result<String, String> {
                match o.get(key) {
                    None | Some(serde_json::Value::Null) => Ok(String::new()),
                    Some(serde_json::Value::String(s)) => Ok(s.trim().to_string()),
                    Some(_) => Err(format!("{at}: `{key}` must be text")),
                }
            };
            let hex = text("hex")?;
            let weight = text("weight")?;
            let spec = ThreadSpec {
                brand: text("brand")?,
                code: text("code")?,
                name: text("name")?,
                rgb: parse_hex(&hex).ok_or_else(|| format!("{at}: `{}` is not a colour (#rrggbb)", shown(&hex)))?,
                weight: (!weight.is_empty()).then_some(weight),
            };
            push(&mut threads, spec, &chart, &at)?;
        }
        finish(chart, threads)
    }

    /// The chart as JSON text ([`ThreadChart::parse_json`] reads it back).
    pub fn to_json(&self) -> String {
        let threads: Vec<serde_json::Value> = self
            .threads
            .iter()
            .map(|t| {
                let mut o = serde_json::json!({"brand": t.brand, "code": t.code, "name": t.name, "hex": hex(t.rgb)});
                if let Some(w) = &t.weight {
                    o["weight"] = serde_json::json!(w);
                }
                o
            })
            .collect();
        serde_json::json!({"format": JSON_FORMAT, "name": self.name, "threads": threads}).to_string()
    }
}

fn too_many() -> String {
    format!("the chart has more than {MAX_CHART_THREADS} threads")
}

/// `s` cut short for a message.
fn shown(s: &str) -> String {
    s.chars().take(24).collect()
}

fn text_of(bytes: &[u8]) -> Result<&str, String> {
    if bytes.len() > MAX_CHART_BYTES {
        return Err(format!("the chart is larger than {} MB", MAX_CHART_BYTES >> 20));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the chart is not UTF-8 text")?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text))
}

fn chart_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    check_field("the chart name", name, MAX_CHART_NAME)?;
    Ok(if name.is_empty() { "Thread Chart".to_string() } else { name.to_string() })
}

fn check_field(what: &str, s: &str, max: usize) -> Result<(), String> {
    if s.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if s.chars().any(char::is_control) {
        return Err(format!("{what} has a control character"));
    }
    Ok(())
}

/// Checks `spec` and adds it. `at` names where it came from, for messages; an empty brand becomes
/// `default_brand`.
fn push(threads: &mut Vec<ThreadSpec>, mut spec: ThreadSpec, default_brand: &str, at: &str) -> Result<(), String> {
    if threads.len() >= MAX_CHART_THREADS {
        return Err(too_many());
    }
    if spec.brand.is_empty() {
        spec.brand = default_brand.to_string();
    }
    let err = |e: String| format!("{at}: {e}");
    check_field("the brand", &spec.brand, MAX_BRAND).map_err(err)?;
    check_field("the code", &spec.code, MAX_CODE).map_err(err)?;
    check_field("the name", &spec.name, MAX_NAME).map_err(err)?;
    if let Some(w) = &spec.weight {
        check_field("the weight", w, MAX_WEIGHT).map_err(err)?;
    }
    if spec.code.is_empty() {
        return Err(err("the code is empty".into()));
    }
    if threads.iter().any(|t| t.brand == spec.brand && t.code == spec.code) {
        return Err(err(format!("{} {} is listed twice", spec.brand, spec.code)));
    }
    threads.push(spec);
    Ok(())
}

fn finish(name: String, threads: Vec<ThreadSpec>) -> Result<ThreadChart, String> {
    if threads.is_empty() {
        return Err("the chart has no threads".into());
    }
    Ok(ThreadChart { name, threads })
}

/// `#rrggbb` or `rrggbb`.
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim();
    let h = h.strip_prefix('#').unwrap_or(h);
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// `#rrggbb`.
pub fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// The records of CSV `text` with the line each starts on; blank lines are skipped.
fn csv_rows(text: &str) -> Result<Vec<(usize, Vec<String>)>, String> {
    // A field is refused far past any allowed one, so a hostile file cannot grow a single field.
    const FIELD_CAP: usize = MAX_NAME * 4;
    let mut rows: Vec<(usize, Vec<String>)> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let (mut quoted, mut was_quoted) = (false, false);
    let (mut line, mut start) = (1usize, 1usize);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => {
                    if c == '\n' {
                        line += 1;
                    }
                    field.push(c);
                }
            }
        } else {
            match c {
                '"' if field.is_empty() => {
                    quoted = true;
                    was_quoted = true;
                }
                '"' => return Err(format!("line {line}: a quote inside an unquoted field")),
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    end_row(&mut rows, &mut row, &mut field, was_quoted, start)?;
                    was_quoted = false;
                    line += 1;
                    start = line;
                }
                _ => field.push(c),
            }
        }
        if field.len() > FIELD_CAP {
            return Err(format!("line {line}: a field is far longer than any thread detail"));
        }
    }
    if quoted {
        return Err(format!("line {start}: a quoted field is never closed"));
    }
    end_row(&mut rows, &mut row, &mut field, was_quoted, start)?;
    Ok(rows)
}

/// Ends the record being read; one with a single empty, unquoted field is a blank line.
fn end_row(rows: &mut Vec<(usize, Vec<String>)>, row: &mut Vec<String>, field: &mut String, was_quoted: bool, start: usize) -> Result<(), String> {
    row.push(std::mem::take(field));
    let done = std::mem::take(row);
    let blank = !was_quoted && done.len() == 1 && done.iter().all(|f| f.trim().is_empty());
    if !blank {
        // The header and one row per thread, and one more so the cap is reached rather than passed.
        if rows.len() > MAX_CHART_THREADS {
            return Err(too_many());
        }
        rows.push((start, done));
    }
    Ok(())
}

/// A colour on the hue circle: `h` degrees, `s` and `l` 0..=1.
fn hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp.floor() {
        v if v < 1.0 => (c, x, 0.0),
        v if v < 2.0 => (x, c, 0.0),
        v if v < 3.0 => (0.0, c, x),
        v if v < 4.0 => (0.0, x, c),
        v if v < 5.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    // Clamped to 0..=255 first, so the cast is exact.
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(r), byte(g), byte(b)]
}

/// The brand of [`starter_chart`].
pub const STARTER_BRAND: &str = "VectorCraft";
/// The name of [`starter_chart`].
pub const STARTER_NAME: &str = "VectorCraft Starter";

/// A small original starter set of 24 named colours: black, white and four greys, twelve hues
/// spread round the colour circle, and six common mixed shades. It is generated here, not taken
/// from any maker's range, so it matches no brand's thread: import a chart for that.
pub fn starter_chart() -> ThreadChart {
    const HUES: [(&str, f64); 12] = [
        ("Red", 0.0),
        ("Orange", 30.0),
        ("Yellow", 55.0),
        ("Lime", 90.0),
        ("Green", 125.0),
        ("Spring", 155.0),
        ("Cyan", 185.0),
        ("Azure", 210.0),
        ("Blue", 235.0),
        ("Violet", 270.0),
        ("Magenta", 305.0),
        ("Rose", 340.0),
    ];
    let mut colours: Vec<(&str, [u8; 3])> = vec![
        ("Black", [0, 0, 0]),
        ("Charcoal", [51, 51, 51]),
        ("Grey", [128, 128, 128]),
        ("Silver", [192, 192, 192]),
        ("Pale Grey", [226, 226, 226]),
        ("White", [255, 255, 255]),
    ];
    colours.extend(HUES.iter().map(|&(n, h)| (n, hsl(h, 0.85, 0.48))));
    colours.extend([
        ("Brown", hsl(28.0, 0.6, 0.27)),
        ("Navy", hsl(225.0, 0.7, 0.2)),
        ("Forest", hsl(140.0, 0.6, 0.24)),
        ("Burgundy", hsl(345.0, 0.7, 0.28)),
        ("Pink", hsl(335.0, 0.8, 0.78)),
        ("Sky", hsl(200.0, 0.8, 0.72)),
    ]);
    let threads = colours
        .into_iter()
        .enumerate()
        .map(|(i, (name, rgb))| ThreadSpec { brand: STARTER_BRAND.into(), code: format!("VC{:02}", i + 1), name: name.into(), rgb, weight: None })
        .collect();
    ThreadChart { name: STARTER_NAME.into(), threads }
}

/// One colour of a needle assignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedleUse {
    /// The needle, from 1. `None`: the machine has no needle left, so this colour needs a thread
    /// swap part way through.
    pub needle: Option<usize>,
    pub rgb: [u8; 3],
    pub stitches: usize,
    /// The blocks of the plan sewn with it, in order.
    pub blocks: Vec<usize>,
}

/// The needle each colour of a plan is sewn with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Needles {
    /// One entry per distinct colour, in order of first use.
    pub uses: Vec<NeedleUse>,
    pub warnings: Vec<String>,
}

/// Gives each colour of `plan`, in the order it is first sewn, the next needle from 1 to `needles`
/// (any number when `None`); a colour sewn again keeps its needle. Colours past the count get no
/// needle and a warning.
pub fn assign_needles(plan: &StitchPlan, needles: Option<u8>) -> Needles {
    let mut uses: Vec<NeedleUse> = Vec::new();
    for (i, b) in plan.blocks.iter().enumerate() {
        let stitches = b.steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count();
        match uses.iter_mut().find(|u| u.rgb == b.thread.rgb) {
            Some(u) => {
                u.stitches += stitches;
                u.blocks.push(i);
            }
            None => {
                let next = uses.iter().filter(|u| u.needle.is_some()).count() + 1;
                let needle = needles.is_none_or(|n| next <= usize::from(n)).then_some(next);
                uses.push(NeedleUse { needle, rgb: b.thread.rgb, stitches, blocks: vec![i] });
            }
        }
    }
    let mut warnings = Vec::new();
    let over = uses.iter().filter(|u| u.needle.is_none()).count();
    if let (Some(n), true) = (needles, over > 0) {
        warnings.push(format!("the design has {} thread colours and the machine {n} needles: {over} need a thread swap during the job", uses.len()));
    }
    Needles { uses, warnings }
}

#[cfg(test)]
mod tests;
