//! PES and PEC machine files (Brother). A PEC block holds what the machine sews: a 512-byte
//! header with the thread palette indices, the stitch block of one- or two-byte moves, and
//! one-bit thumbnails. A PES file wraps it: `#PES0001`, the PEC block's offset, and a design
//! section (`CEmbOne`, `CSewSeg`) that editing software reads. A `.pec` file is `#PEC0001` and the
//! PEC block. See `docs/superpowers/specs/2026-11-13-embroidery-pes-format.md`.
//!
//! The reader takes the PEC block of any PES version; it does not read the design section (or the
//! thread list newer versions keep there).

mod threads;

use crate::dst::header::label;
use crate::{Builder, Input, MAX_FILE_BYTES, Output, ReadOptions, Sink, TOO_LONG, WriteOptions, default_thread, walk};
use vectorcraft_stitch::{Step, StitchPlan};

/// The start of every PES file; the version follows.
const PES: &[u8] = b"#PES";
/// The version this writer writes.
const PES_V1: &[u8] = b"#PES0001";
/// The start of a PEC file.
const PEC: &[u8] = b"#PEC0001";
/// The largest move per axis: twelve bits, signed.
const MAX_MOVE: i32 = 2047;
/// A move of this size or more, either way, takes the two-byte form: one byte holds -63 to 62.
const SHORT: i32 = 63;
const LONG: u8 = 0x80;
const JUMP: u8 = 0x10;
const TRIM: u8 = 0x20;
const COLOR_CHANGE: [u8; 2] = [0xFE, 0xB0];
const END: u8 = 0xFF;
/// The PEC header's length, before the stitch block.
const PEC_HEADER: usize = 512;
/// Where the stitches start: the stitch block's own 16-byte header follows the PEC header.
const STITCHES_AT: usize = PEC_HEADER + 16;
/// The thumbnails: 48 x 38 pixels, one bit each, six bytes a row.
const ICON_W: i64 = 48;
const ICON_H: i64 = 38;
const STRIDE: usize = 6;
const ICON: usize = STRIDE * 38;
/// The most colour blocks a PEC header lists.
const MAX_COLOURS: usize = 255;
/// The most points one design-section segment holds.
const MAX_SEGMENT: usize = 32_767;

/// Whether `bytes` start like a PES or PEC file.
pub fn is_pes(bytes: &[u8]) -> bool {
    bytes.starts_with(PES) || bytes.starts_with(PEC)
}

/// A record, with its move in PEC orientation (y down).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ev {
    Stitch(i32, i32),
    Jump(i32, i32),
    Trim,
    Color,
}

/// Records as the walk hands them over.
#[derive(Default)]
struct Events(Vec<Ev>);

fn fits(dx: i32, dy: i32) -> Result<(), String> {
    if dx.abs() > MAX_MOVE || dy.abs() > MAX_MOVE { Err(TOO_LONG.to_string()) } else { Ok(()) }
}

impl Sink for Events {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        fits(dx, dy)?;
        self.0.push(Ev::Stitch(dx, -dy));
        Ok(())
    }

    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        fits(dx, dy)?;
        self.0.push(Ev::Jump(dx, -dy));
        Ok(())
    }

    fn trim(&mut self) -> Result<(), String> {
        self.0.push(Ev::Trim);
        Ok(())
    }

    fn color_change(&mut self) -> Result<(), String> {
        self.0.push(Ev::Color);
        Ok(())
    }
}

/// A plan as PEC records, with what the headers need.
struct Design {
    /// Each record and the position it reaches (y down, from the origin).
    ev: Vec<(Ev, i64, i64)>,
    /// The colour of each colour block in sewing order: a plan's stop is a change to the same
    /// colour, as the format has no stop.
    colours: Vec<[u8; 3]>,
    /// `[min x, min y, max x, max y]` of the stitches and jumps (y down).
    bounds: [i64; 4],
}

fn design(plan: &StitchPlan, o: &WriteOptions) -> Result<Design, String> {
    let mut sink = Events::default();
    walk(plan, o, MAX_MOVE, &mut sink)?;
    let mut colours = Vec::new();
    for b in &plan.blocks {
        colours.push(b.thread.rgb);
        colours.extend(b.steps.iter().filter(|s| **s == Step::Stop).map(|_| b.thread.rgb));
    }
    if colours.len() > MAX_COLOURS {
        return Err(format!("a PES file holds at most {MAX_COLOURS} colour blocks"));
    }
    let (mut x, mut y) = (0i64, 0i64);
    let mut bounds: Option<[i64; 4]> = None;
    let mut ev = Vec::with_capacity(sink.0.len());
    for e in sink.0 {
        if let Ev::Stitch(dx, dy) | Ev::Jump(dx, dy) = e {
            x += i64::from(dx);
            y += i64::from(dy);
            bounds = Some(match bounds {
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
                None => [x, y, x, y],
            });
        }
        ev.push((e, x, y));
    }
    Ok(Design { ev, colours, bounds: bounds.unwrap_or_default() })
}

fn i16_le(v: i64) -> [u8; 2] {
    i16::try_from(v.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0).to_le_bytes()
}

/// One coordinate: a byte for -63 to 62 unless `long`, else two bytes, the first with bit 0x80
/// and `flag`, holding twelve bits.
fn value(out: &mut Vec<u8>, v: i32, long: bool, flag: u8) {
    if !long && -SHORT < v && v < SHORT {
        let [b] = i8::try_from(v).unwrap_or(0).to_le_bytes();
        out.push(b & 0x7F);
    } else {
        let twelve = u16::try_from(v & 0x0FFF).unwrap_or(0);
        let [hi, lo] = (twelve | (u16::from(LONG | flag) << 8)).to_be_bytes();
        out.extend([hi, lo]);
    }
}

/// The stitch records: stitches short or long as they fit, jumps long with the jump flag, or the
/// trim flag on the first jump after a trim (a trim with no jump after it is a trim of no move),
/// colour changes `FE B0` then 2, 1, 2 ..., and the end `FF`. Returns the bytes and the count.
fn stitches(d: &Design) -> (Vec<u8>, usize) {
    /// A pending trim with no jump to carry it: a trim of no move.
    fn cut(out: &mut Vec<u8>, records: &mut usize, trim: &mut bool) {
        if std::mem::take(trim) {
            value(out, 0, true, TRIM);
            value(out, 0, true, TRIM);
            *records += 1;
        }
    }
    let mut out = Vec::new();
    let mut records = 0;
    let mut trim = false;
    let mut two = true;
    for &(e, _, _) in &d.ev {
        match e {
            Ev::Stitch(dx, dy) => {
                cut(&mut out, &mut records, &mut trim);
                value(&mut out, dx, false, 0);
                value(&mut out, dy, false, 0);
            }
            Ev::Jump(dx, dy) => {
                let flag = if std::mem::take(&mut trim) { TRIM } else { JUMP };
                value(&mut out, dx, true, flag);
                value(&mut out, dy, true, flag);
            }
            Ev::Trim => {
                trim = true;
                continue;
            }
            Ev::Color => {
                cut(&mut out, &mut records, &mut trim);
                out.extend(COLOR_CHANGE);
                out.push(if two { 2 } else { 1 });
                two = !two;
            }
        }
        records += 1;
    }
    cut(&mut out, &mut records, &mut trim);
    out.push(END);
    (out, records + 1)
}

/// The thumbnail frame: a rounded rectangle one pixel in from the edge.
fn frame() -> [u8; ICON] {
    let mut g = [0u8; ICON];
    for x in 4..=43 {
        mark(&mut g, x, 1);
        mark(&mut g, x, 36);
    }
    for (d, (y0, y1)) in [(3, (2, 35)), (2, (3, 34))] {
        mark(&mut g, d, y0);
        mark(&mut g, ICON_W - 1 - d, y0);
        mark(&mut g, d, y1);
        mark(&mut g, ICON_W - 1 - d, y1);
    }
    for y in 4..=33 {
        mark(&mut g, 1, y);
        mark(&mut g, ICON_W - 2, y);
    }
    g
}

/// Sets pixel (`x`, `y`), the lowest bit of a byte leftmost; outside the icon, nothing.
fn mark(g: &mut [u8; ICON], x: i64, y: i64) {
    if !(0..ICON_W).contains(&x) || !(0..ICON_H).contains(&y) {
        return;
    }
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return };
    if let Some(b) = g.get_mut(y * STRIDE + x / 8) {
        *b |= 1 << (x % 8);
    }
}

/// A thumbnail of `points` in the frame, scaled to the design's bounds less `margin` pixels and
/// centred.
fn icon(bounds: [i64; 4], points: impl Iterator<Item = (i64, i64)>, margin: f64) -> [u8; ICON] {
    let mut g = frame();
    let [x0, y0, x1, y1] = bounds.map(|v| v as f64);
    let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    let scale = ((ICON_W as f64 - margin) / w).min((ICON_H as f64 - margin) / h);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    for (x, y) in points {
        let px = ((x as f64 - cx) * scale + ICON_W as f64 / 2.0).floor();
        let py = ((y as f64 - cy) * scale + ICON_H as f64 / 2.0).floor();
        if px.is_finite() && py.is_finite() {
            mark(&mut g, px as i64, py as i64);
        }
    }
    g
}

/// The PEC block: header, stitch block and thumbnails (the whole design, then one per colour
/// block).
fn pec(name: &str, d: &Design) -> Result<(Vec<u8>, usize), String> {
    let n = d.colours.len();
    let last =
        u8::try_from(n.checked_sub(1).ok_or("nothing to stitch")?).map_err(|_| format!("a PES file holds at most {MAX_COLOURS} colour blocks"))?;
    let mut out = b"LA:".to_vec();
    let mut name: Vec<u8> = label(name).bytes().take(8).collect();
    name.resize(16, b' ');
    out.extend(name);
    out.push(b'\r');
    out.extend([b' '; 12]);
    out.extend([0xFF, 0x00]);
    out.extend([STRIDE as u8, ICON_H as u8]);
    out.extend([b' '; 12]);
    out.push(last);
    out.extend(threads::unique_indices(&d.colours));
    out.resize(PEC_HEADER, b' ');
    let [x0, y0, x1, y1] = d.bounds;
    let start = out.len();
    out.extend([0, 0, 0, 0, 0, 0x31, 0xFF, 0xF0]);
    out.extend(i16_le(x1 - x0));
    out.extend(i16_le(y1 - y0));
    out.extend(0x1E0u16.to_le_bytes());
    out.extend(0x1B0u16.to_le_bytes());
    let (records, count) = stitches(d);
    out.extend(records);
    let len = u32::try_from(out.len() - start).ok().filter(|l| *l < 1 << 24).ok_or("the design is too large for a PEC stitch block")?;
    let [a, b, c, _] = len.to_le_bytes();
    if let Some(slot) = out.get_mut(start + 2..start + 5) {
        slot.copy_from_slice(&[a, b, c]);
    }
    let stitched = |block: Option<usize>| {
        let mut k = 0;
        d.ev.iter().filter_map(move |&(e, x, y)| match e {
            Ev::Color => {
                k += 1;
                None
            }
            Ev::Stitch(..) if block.is_none_or(|b| b == k) => Some((x, y)),
            _ => None,
        })
    };
    out.extend(icon(d.bounds, stitched(None), 4.0));
    for b in 0..n {
        out.extend(icon(d.bounds, stitched(Some(b)), 5.0));
    }
    Ok((out, count))
}

/// A string with its length as a 16-bit number first.
fn string16(out: &mut Vec<u8>, s: &str) {
    out.extend(i16_le(s.len() as i64));
    out.extend(s.bytes());
}

/// The design section: the `CEmbOne` object (its placement in a 130 x 180 mm hoop) and the
/// `CSewSeg` segments (each run of stitches, each jump as its two ends, each with its palette
/// entry), then the colour log (the first segment of each colour). Coordinates are from the
/// design's left and bottom edges, y down.
fn design_section(d: &Design) -> Vec<u8> {
    let mut out = Vec::new();
    let [x0, y0, x1, y1] = d.bounds;
    let (w, h) = (x1 - x0, y1 - y0);
    string16(&mut out, "CEmbOne");
    out.extend([0u8; 16]);
    let tx = 350.0 + 1300.0 / 2.0 - w as f32 / 2.0;
    let ty = 100.0 + h as f32 + 1800.0 / 2.0 - h as f32 / 2.0;
    for f in [1.0f32, 0.0, 0.0, 1.0, tx, ty] {
        out.extend(f.to_le_bytes());
    }
    for v in [1, 0, 0, w, h] {
        out.extend(i16_le(v));
    }
    out.extend([0u8; 8]);
    let sections_at = out.len();
    out.extend([0, 0]);
    out.extend(0xFFFFu16.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    string16(&mut out, "CSewSeg");
    let codes: Vec<i64> = d.colours.iter().map(|c| threads::nearest(*c, |_| true).map_or(0, i64::from)).collect();
    let mut colour = 0usize;
    let mut stitched = (0i64, 0i64);
    let mut sections = 0i64;
    let mut log: Vec<(i64, i64)> = Vec::new();
    let mut segment = |out: &mut Vec<u8>, flag: i64, code: i64, points: &[(i64, i64)]| {
        if sections > 0 {
            out.extend(0x8003u16.to_le_bytes());
        }
        if log.last().is_none_or(|(_, c)| *c != code) {
            log.push((sections, code));
        }
        for v in [flag, code, points.len() as i64] {
            out.extend(i16_le(v));
        }
        for (x, y) in points {
            out.extend(i16_le(x - x0));
            out.extend(i16_le(y - y1));
        }
        sections += 1;
    };
    let mut i = 0;
    while let Some(&(e, ..)) = d.ev.get(i) {
        let same = |f: &(Ev, i64, i64)| std::mem::discriminant(&f.0) == std::mem::discriminant(&e);
        let run: Vec<(i64, i64)> = d.ev.get(i..).unwrap_or_default().iter().take_while(|f| same(f)).map(|f| (f.1, f.2)).collect();
        i += run.len();
        let code = codes.get(colour).copied().unwrap_or(0);
        match e {
            Ev::Stitch(..) => {
                for chunk in run.chunks(MAX_SEGMENT) {
                    segment(&mut out, 0, code, chunk);
                }
                stitched = run.last().copied().unwrap_or(stitched);
            }
            Ev::Jump(..) => {
                let end = run.last().copied().unwrap_or(stitched);
                segment(&mut out, 1, code, &[stitched, end]);
            }
            Ev::Color => colour += run.len(),
            Ev::Trim => {}
        }
    }
    if let Some(slot) = out.get_mut(sections_at..sections_at + 2) {
        slot.copy_from_slice(&i16_le(sections));
    }
    out.extend(i16_le(log.len() as i64));
    for (s, c) in log {
        out.extend(i16_le(s));
        out.extend(i16_le(c));
    }
    out.extend([0u8; 4]);
    out
}

/// The PES file (version 1) for `plan`, which must already fit the format (see
/// [`crate::WriteOptions`]; `jumps_per_trim` does not apply: a trim is a flag on the jump after
/// it). Thread colours become the nearest entries of the PEC palette.
pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    let d = design(plan, o)?;
    let (pec, records) = pec(&plan.name, &d)?;
    let mut bytes = PES_V1.to_vec();
    let offset_at = bytes.len();
    bytes.extend([0u8; 4]);
    for v in [1, 1, 1] {
        bytes.extend(i16_le(v));
    }
    bytes.extend(0xFFFFu16.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(design_section(&d));
    let offset = u32::try_from(bytes.len()).map_err(|_| "the design is too large".to_string())?;
    if let Some(slot) = bytes.get_mut(offset_at..offset_at + 4) {
        slot.copy_from_slice(&offset.to_le_bytes());
    }
    bytes.extend(pec);
    Ok(Output { bytes, records, warnings: Vec::new() })
}

/// The PEC file for `plan`: `#PEC0001` and the PEC block, as [`write`] otherwise.
pub fn write_pec(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    let d = design(plan, o)?;
    let (pec, records) = pec(&plan.name, &d)?;
    let mut bytes = PEC.to_vec();
    bytes.extend(pec);
    Ok(Output { bytes, records, warnings: Vec::new() })
}

/// One coordinate at `s[i..]`: its value, its length and its flags. `None` when the file ends.
fn coordinate(s: &[u8], i: usize) -> Option<(i32, usize, u8)> {
    let b = *s.get(i)?;
    if b & LONG == 0 {
        let v = i32::from(b);
        return Some((if v > 63 { v - 128 } else { v }, 1, 0));
    }
    let lo = *s.get(i + 1)?;
    let v = (i32::from(b) << 8 | i32::from(lo)) & 0x0FFF;
    Some((if v > 0x7FF { v - 0x1000 } else { v }, 2, b & (JUMP | TRIM)))
}

/// The stitch plan in a PES (any version) or PEC file, y down, with warnings for what could not
/// be read. Every colour change starts a new block, coloured from the PEC palette.
pub fn read(bytes: &[u8], _o: &ReadOptions) -> Result<Input, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is larger than 64 MB".to_string());
    }
    let at = if bytes.starts_with(PEC) {
        PEC.len()
    } else if bytes.starts_with(PES) {
        let o: [u8; 4] = bytes.get(8..12).and_then(|b| b.try_into().ok()).ok_or("not a PES file: the header is incomplete")?;
        usize::try_from(u32::from_le_bytes(o)).map_err(|_| "not a PES file: the PEC offset is out of range".to_string())?
    } else {
        return Err("not a PES or PEC file".to_string());
    };
    let p = bytes.get(at..).filter(|p| p.len() >= STITCHES_AT).ok_or("the PEC block is missing or incomplete")?;
    let name: String = p
        .get(3..19)
        .unwrap_or_default()
        .iter()
        .take_while(|c| **c != b'\r')
        .map(|c| if c.is_ascii_graphic() || *c == b' ' { char::from(*c) } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string();
    let count = p.get(48).map_or(0, |c| if *c == 0xFF { 0 } else { usize::from(*c) + 1 });
    let threads = p
        .get(49..49 + count)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, idx)| threads::thread(*idx).unwrap_or_else(|| default_thread(i)))
        .collect();
    let len = p.get(PEC_HEADER + 2..PEC_HEADER + 5).map_or(0, |b| b.iter().rev().fold(0usize, |a, v| a << 8 | usize::from(*v)));
    let end = (PEC_HEADER + len).min(p.len()).max(STITCHES_AT);
    let s = p.get(STITCHES_AT..end).unwrap_or_default();
    let mut b = Builder::new(threads, None);
    let mut warnings = Vec::new();
    let mut ended = false;
    let mut i = 0;
    while let Some(&first) = s.get(i) {
        if first == END {
            ended = true;
            break;
        }
        if s.get(i..i + 2) == Some(&COLOR_CHANGE[..]) {
            i += 3;
            b.color_change(0, 0)?;
            continue;
        }
        b.count()?;
        let Some((x, n1, f1)) = coordinate(s, i) else { break };
        let Some((y, n2, f2)) = coordinate(s, i + n1) else { break };
        i += n1 + n2;
        let flags = f1 | f2;
        if flags & JUMP != 0 {
            b.jump(x, -y);
        } else if flags & TRIM != 0 {
            b.trim();
            if (x, y) != (0, 0) {
                b.jump(x, -y);
            }
        } else {
            b.stitch(x, -y);
        }
    }
    if !ended {
        warnings.push(if i < s.len() { "the file ends in the middle of a stitch" } else { "the file has no end record" }.to_string());
    }
    Ok(Input { plan: b.finish(name), needles: Vec::new(), warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Start;
    use proptest::prelude::*;
    use vectorcraft_stitch::{Block, Pt, Thread};

    fn tiny() -> StitchPlan {
        StitchPlan {
            name: "TEST".into(),
            blocks: vec![Block {
                thread: Thread { rgb: [0, 0, 0], ..Thread::default() },
                steps: vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 10, y: 0 }), Step::Stitch(Pt { x: 10, y: 100 })],
            }],
        }
    }

    fn first_only() -> WriteOptions {
        WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() }
    }

    /// The PEC block of a PES file.
    fn pec_of(b: &[u8]) -> &[u8] {
        let at = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        &b[at..]
    }

    #[test]
    fn a_tiny_design_has_exact_pec_bytes() {
        let out = write(&tiny(), &first_only()).unwrap();
        assert!(out.bytes.starts_with(b"#PES0001"));
        assert!(is_pes(&out.bytes));
        let p = pec_of(&out.bytes);
        assert_eq!(&p[..20], b"LA:TEST            \r");
        assert_eq!(&p[20..36], b"            \xff\x00\x06\x26");
        assert_eq!(&p[48..50], [0, 20], "one colour: black, entry 20");
        assert!(p[50..512].iter().all(|c| *c == b' '));
        // The stitch block: its length, the fixed bytes, width 10 and height 100.
        let records = [0x00, 0x00, 0x0A, 0x00, 0x00, 0x80, 0x64, 0xFF];
        let len = 16 + records.len();
        assert_eq!(&p[512..528], [0, 0, len as u8, 0, 0, 0x31, 0xFF, 0xF0, 10, 0, 100, 0, 0xE0, 0x01, 0xB0, 0x01]);
        assert_eq!(&p[528..528 + records.len()], records, "in place, right 10, down 100 (long form)");
        assert_eq!(p.len(), 512 + len + 2 * ICON);
        assert_eq!(out.records, 4);
        // The PEC file is the same block.
        let pec = write_pec(&tiny(), &first_only()).unwrap().bytes;
        assert_eq!((&pec[..8], &pec[8..]), (&b"#PEC0001"[..], p));
    }

    #[test]
    fn jumps_trims_and_colour_changes_have_their_codes() {
        let mut p = tiny();
        p.blocks[0].steps.extend([Step::Jump(Pt { x: 20, y: 100 }), Step::Stitch(Pt { x: 20, y: 100 }), Step::Trim, Step::Jump(Pt { x: 30, y: 90 })]);
        p.blocks[0].steps.push(Step::Stitch(Pt { x: 30, y: 90 }));
        p.blocks.push(Block { thread: Thread { rgb: [240, 240, 240], ..Thread::default() }, steps: vec![Step::Stitch(Pt { x: 30, y: 80 })] });
        p.blocks.push(Block { thread: Thread { rgb: [0, 0, 0], ..Thread::default() }, steps: vec![Step::Stitch(Pt { x: 30, y: 70 })] });
        let out = write(&p, &first_only()).unwrap();
        let pec = pec_of(&out.bytes);
        assert_eq!(&pec[48..52], [2, 20, 29, 20], "black, white, black");
        let s = &pec[528..];
        let want = [
            0x00, 0x00, 0x0A, 0x00, 0x00, 0x80, 0x64, // stitches
            0x90, 0x0A, 0x90, 0x00, 0x00, 0x00, // a jump of 10, then a stitch in place
            0xA0, 0x0A, 0xAF, 0xF6, 0x00, 0x00, // a trim and jump of (10, -10), a stitch
            0xFE, 0xB0, 0x02, 0x00, 0x76, // colour change, up 10
            0xFE, 0xB0, 0x01, 0x00, 0x76, // colour change, up 10
            0xFF,
        ];
        assert_eq!(&s[..want.len()], want);
        let back = read(&out.bytes, &ReadOptions::default()).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.plan.name, "TEST");
        assert_eq!(back.plan.blocks.len(), 3);
        assert_eq!(back.plan.blocks[1].thread.name.as_deref(), Some("White"));
        assert_eq!(back.plan.blocks[0].thread.brand.as_deref(), Some("Brother"));
        assert_eq!(back.plan.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1);
        // A trim with no jump after it is a trim of no move; a stop is a change to the same colour.
        let mut q = tiny();
        q.blocks[0].steps.extend([Step::Trim, Step::Stop, Step::Stitch(Pt { x: 0, y: 0 })]);
        let out = write(&q, &first_only()).unwrap();
        let pec = pec_of(&out.bytes);
        assert_eq!(&pec[48..51], [1, 20, 20]);
        assert_eq!(&pec[528 + 7..528 + 14], [0xA0, 0x00, 0xA0, 0x00, 0xFE, 0xB0, 0x02]);
    }

    #[test]
    fn the_design_section_lists_segments_and_colours() {
        let mut p = tiny();
        p.blocks.push(Block {
            thread: Thread { rgb: [240, 240, 240], ..Thread::default() },
            steps: vec![Step::Jump(Pt { x: 50, y: 50 }), Step::Stitch(Pt { x: 50, y: 50 })],
        });
        let b = write(&p, &first_only()).unwrap().bytes;
        assert_eq!(&b[12..22], [1, 0, 1, 0, 1, 0, 0xFF, 0xFF, 0, 0]);
        assert_eq!(&b[22..31], b"\x07\x00CEmbOne");
        // Three segments: the first block's stitches, the jump, the second block's stitch.
        let at = 31 + 16 + 24 + 10 + 8;
        assert_eq!(&b[at..at + 2], [3, 0]);
        let seg = at + 2 + 4 + 9;
        assert_eq!(&b[seg - 9..seg], b"\x07\x00CSewSeg");
        // Stitch segment, black (20), three points from the left and bottom.
        assert_eq!(&b[seg..seg + 6], [0, 0, 20, 0, 3, 0]);
        assert_eq!(&b[seg + 6..seg + 18], [0, 0, 0x9C, 0xFF, 10, 0, 0x9C, 0xFF, 10, 0, 0, 0]);
        assert!(b.windows(2).any(|w| w == [0x03, 0x80]), "segments are separated by 0x8003");
        // The colour log: black from segment 0, white (29) from segment 1, the jump to it.
        let pec = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        assert_eq!(&b[pec - 14..pec], [2, 0, 0, 0, 20, 0, 1, 0, 29, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn odd_files_are_survived() {
        assert_eq!(read(b"hello", &ReadOptions::default()).unwrap_err(), "not a PES or PEC file");
        assert_eq!(read(b"#PES0001\x01", &ReadOptions::default()).unwrap_err(), "not a PES file: the header is incomplete");
        assert_eq!(read(b"#PES0001\xff\xff\xff\x7f", &ReadOptions::default()).unwrap_err(), "the PEC block is missing or incomplete");
        let mut b = write_pec(&tiny(), &first_only()).unwrap().bytes;
        let end = 8 + 528 + 7;
        assert_eq!(b[end], 0xFF);
        b.truncate(end);
        b.push(0x85);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count(), 3);
        assert_eq!(i.warnings, vec!["the file ends in the middle of a stitch".to_string()]);
        // Garbage colour indices fall back to the built-in list; a length past the end is held to
        // the file.
        let mut b = write_pec(&tiny(), &first_only()).unwrap().bytes;
        b[8 + 49] = 200;
        b[8 + 514] = 0xFF;
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].thread, default_thread(0));
        assert!(i.warnings.is_empty(), "{:?}", i.warnings);
        assert_eq!(read(&vec![0u8; MAX_FILE_BYTES + 1], &ReadOptions::default()).unwrap_err(), "the file is larger than 64 MB");
    }

    #[test]
    fn limits() {
        let mut long = tiny();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 10 + 2048, y: 100 }));
        assert_eq!(write(&long, &first_only()).err().unwrap(), TOO_LONG);
        let mut many = tiny();
        for _ in 0..MAX_COLOURS {
            many.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 0, y: 0 })] });
        }
        assert_eq!(write(&many, &first_only()).err().unwrap(), "a PES file holds at most 255 colour blocks");
        many.blocks.pop();
        assert!(write(&many, &first_only()).is_ok());
        let mut b = PEC.to_vec();
        b.extend(vec![b' '; 512]);
        b.extend([0, 0, 0, 0, 1, 0x31, 0xFF, 0xF0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for _ in 0..crate::MAX_BLOCKS {
            b.extend([0x01, 0x00, 0xFE, 0xB0, 0x02]);
        }
        assert_eq!(read(&b, &ReadOptions::default()).unwrap_err(), "the file has too many colour changes");
    }

    #[test]
    fn the_frame_is_drawn_in_code_and_icons_mark_the_stitches() {
        let f = frame();
        assert_eq!(f[0..6], [0; 6]);
        assert_eq!(f[6..12], [0xF0, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        let empty = icon([0, 0, 10, 10], std::iter::empty(), 5.0);
        assert_eq!(empty, f);
        let one = icon([0, 0, 10, 10], [(5, 5)].into_iter(), 5.0);
        assert_eq!(one[19 * STRIDE + 3], f[19 * STRIDE + 3] | 1);
    }

    /// A plan whose moves fit the format, with no stop (PES has none) and no two jumps in a row.
    fn arb_plan() -> impl Strategy<Value = StitchPlan> {
        let op = prop_oneof![
            6 => (-121i32..=121, -121i32..=121).prop_map(|(x, y)| (0u8, x, y)),
            2 => (-500i32..=500, -500i32..=500).prop_filter("a jump that moves", |(x, y)| (*x, *y) != (0, 0)).prop_map(|(x, y)| (1u8, x, y)),
            2 => Just((2u8, 0, 0)),
        ];
        let colour = prop_oneof![Just([0u8, 0, 0]), Just([240, 240, 240]), Just([237, 23, 31]), Just([10, 85, 163])];
        let block = (prop::collection::vec(op, 0..40), -121i32..=121, -121i32..=121, colour);
        (prop::collection::vec(block, 1..5), -500i32..500, -500i32..500).prop_map(|(blocks, x0, y0)| {
            let mut at = Pt { x: x0, y: y0 };
            let mut out = Vec::new();
            for (ops, fx, fy, rgb) in blocks {
                at = Pt { x: at.x + fx, y: at.y + fy };
                let mut steps = vec![Step::Stitch(at)];
                for (kind, dx, dy) in ops {
                    match kind {
                        0 => {
                            at = Pt { x: at.x + dx, y: at.y + dy };
                            steps.push(Step::Stitch(at));
                        }
                        1 if !matches!(steps.last(), Some(Step::Jump(_))) => {
                            at = Pt { x: at.x + dx, y: at.y + dy };
                            steps.push(Step::Jump(at));
                        }
                        1 => {}
                        // Two trims in a row are one cut.
                        _ if steps.last() == Some(&Step::Trim) => {}
                        _ => steps.push(Step::Trim),
                    }
                }
                out.push(Block { thread: Thread { rgb, ..Thread::default() }, steps });
            }
            StitchPlan { name: "P".into(), blocks: out }
        })
    }

    fn relative(p: &StitchPlan) -> Vec<Vec<Step>> {
        let f = p.blocks.iter().flat_map(|b| b.steps.iter()).find_map(|s| if let Step::Stitch(q) = s { Some(*q) } else { None }).unwrap_or_default();
        let shift = |q: Pt| Pt { x: q.x - f.x, y: q.y - f.y };
        p.blocks
            .iter()
            .map(|b| {
                b.steps
                    .iter()
                    .map(|s| match *s {
                        Step::Stitch(q) => Step::Stitch(shift(q)),
                        Step::Jump(q) => Step::Jump(shift(q)),
                        other => other,
                    })
                    .collect()
            })
            .collect()
    }

    proptest! {
        #[test]
        fn write_then_read_keeps_stitches_jumps_trims_and_colours(plan in arb_plan(), centre in any::<bool>(), pec in any::<bool>()) {
            let start = if centre { Start::Center } else { Start::FirstStitch };
            let o = WriteOptions { start, return_to_start: false, ..Default::default() };
            let out = if pec { write_pec(&plan, &o) } else { write(&plan, &o) }.unwrap();
            let back = read(&out.bytes, &ReadOptions::default()).unwrap();
            prop_assert!(back.warnings.is_empty(), "{:?}", back.warnings);
            let mut got = relative(&back.plan);
            if let Some(first) = got.first_mut().filter(|b| matches!(b.first(), Some(Step::Jump(_)))) {
                first.remove(0);
            }
            prop_assert_eq!(got, relative(&plan));
            // Colours that are palette entries come back as they were.
            let rgb = |p: &StitchPlan| p.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>();
            prop_assert_eq!(rgb(&back.plan), rgb(&plan));
        }
    }
}
