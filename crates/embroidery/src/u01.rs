//! U01 machine files (Barudan): a 256-byte header, then three-byte records
//! `[control, |dy|, |dx|]` with needle numbers. See
//! `docs/superpowers/specs/2026-11-13-embroidery-u01-format.md`.
//!
//! U01 has no signature: a file is recognised by its `.u01` extension, as DSB is.

use crate::{Builder, Input, MAX_FILE_BYTES, MAX_NEEDLE, Output, ReadOptions, Sink, TOO_LONG, WriteOptions, default_thread, needle, walk};
use vectorcraft_stitch::StitchPlan;

/// The header's length: 128 ASCII `0`s, then 128 bytes of binary fields and zeros.
const HEADER: usize = 0x100;
/// The largest move the writer writes (a record holds 255 per axis, one byte): travel is split into jumps of at most 12.7 mm, the longest
/// stitch, as pyembroidery's U01 writer splits it.
const MAX_WRITE: i32 = 127;
/// Bit 0x80 is set on every record; the low five bits say what a record is.
const BASE: u8 = 0x80;
const KIND: u8 = 0x1F;
/// Set when the move on that axis is zero or negative (left, or down with y up). The codes of the
/// records without a move (trim 0xE7, stop 0xE8, needles 0xE9 on, end 0xF8) carry both.
const X_NEG: u8 = 0x20;
const Y_NEG: u8 = 0x40;
const STITCH: u8 = 0x00;
const JUMP: u8 = 0x01;
/// Fast and slow stitches and jumps (2 to 5): read as stitches and jumps; never written.
const FAST_SLOW: [u8; 4] = [0x02, 0x03, 0x04, 0x05];
/// T1 (top thread) and T2 (both threads) trims; T2 is the one written.
const TRIM_T1: u8 = 0x06;
const TRIM: u8 = 0x07;
/// C00: stop the machine.
const STOP: u8 = 0x08;
/// C01..C15: change to needle n is `NEEDLE_BASE + n`.
const NEEDLE_BASE: u8 = 0x08;
/// The end of the file.
const END: u8 = 0x18;

/// The record of kind `kind` for a move of (`dx`, `dy`), y up. `None` beyond +-255.
fn encode(kind: u8, dx: i32, dy: i32) -> Option<[u8; 3]> {
    let x = u8::try_from(dx.checked_abs()?).ok()?;
    let y = u8::try_from(dy.checked_abs()?).ok()?;
    let ctrl = BASE | kind | if dx <= 0 { X_NEG } else { 0 } | if dy <= 0 { Y_NEG } else { 0 };
    Some([ctrl, y, x])
}

/// The move a record makes, y up.
fn decode(r: [u8; 3]) -> (i32, i32) {
    let [ctrl, y, x] = r;
    let (x, y) = (i32::from(x), i32::from(y));
    (if ctrl & X_NEG != 0 { -x } else { x }, if ctrl & Y_NEG != 0 { -y } else { y })
}

/// Records as they are written, with the position and extents they reach (y up, from the origin).
struct Records<'a> {
    o: &'a WriteOptions,
    out: Vec<[u8; 3]>,
    at: (i64, i64),
    /// `[min x, min y, max x, max y]`.
    ext: [i64; 4],
    /// The block being written.
    block: usize,
}

impl Records<'_> {
    fn push(&mut self, kind: u8, dx: i32, dy: i32) -> Result<(), String> {
        let r = encode(kind, dx, dy).ok_or(TOO_LONG)?;
        self.out.push(r);
        self.at = (self.at.0 + i64::from(dx), self.at.1 + i64::from(dy));
        let [x0, y0, x1, y1] = self.ext;
        self.ext = [x0.min(self.at.0), y0.min(self.at.1), x1.max(self.at.0), y1.max(self.at.1)];
        Ok(())
    }

    fn needle(&mut self) -> Result<(), String> {
        self.push(NEEDLE_BASE + needle(self.o, self.block), 0, 0)
    }
}

impl Sink for Records<'_> {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.push(STITCH, dx, dy)
    }

    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.push(JUMP, dx, dy)
    }

    fn trim(&mut self) -> Result<(), String> {
        self.push(TRIM, 0, 0)
    }

    fn color_change(&mut self) -> Result<(), String> {
        self.block += 1;
        self.needle()
    }

    /// The file sets the first block's needle before anything else.
    fn begin(&mut self) -> Result<(), String> {
        self.needle()
    }

    fn stop(&mut self) -> Option<Result<(), String>> {
        Some(self.push(STOP, 0, 0))
    }
}

/// The 256-byte header: 128 ASCII `0`s; then, little-endian, the extents (left, bottom, right,
/// top, y up, from the origin), a zero `u32`, the record count (the end record included) and the
/// end point; then zeros.
fn header(r: &Records, records: usize) -> Vec<u8> {
    let i16_of = |v: i64| i16::try_from(v.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0).to_le_bytes();
    let mut h = vec![b'0'; 0x80];
    for v in r.ext {
        h.extend(i16_of(v));
    }
    h.extend(0u32.to_le_bytes());
    h.extend(u32::try_from(records).unwrap_or(u32::MAX).to_le_bytes());
    h.extend(i16_of(r.at.0));
    h.extend(i16_of(r.at.1));
    h.resize(HEADER, 0);
    h
}

/// The U01 file for `plan`, which must already fit the format (see [`crate::WriteOptions`];
/// `jumps_per_trim` does not apply: a trim has its own code).
pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    let mut r = Records { o, out: Vec::new(), at: (0, 0), ext: [0; 4], block: 0 };
    walk(plan, o, MAX_WRITE, &mut r)?;
    r.out.push([BASE | X_NEG | Y_NEG | END, 0, 0]);
    let records = r.out.len();
    let mut bytes = header(&r, records);
    bytes.extend(r.out.iter().flatten());
    Ok(Output { bytes, records, warnings: Vec::new() })
}

/// The stitch plan in a U01 file, y down, with warnings for what could not be read. A needle
/// change after stitches, and a stop, start a new block; the needle each block names is kept in
/// [`Input::needles`], and a block's colour comes from the built-in list by its needle (the format
/// stores no colours).
pub fn read(bytes: &[u8], _o: &ReadOptions) -> Result<Input, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is larger than 64 MB".to_string());
    }
    let Some(body) = bytes.get(HEADER..) else {
        return Err("not a U01 file: the header is incomplete".to_string());
    };
    let mut b = Builder::new(Vec::new(), None);
    let mut warnings = Vec::new();
    let mut ended = false;
    for chunk in body.chunks(3) {
        let &[ctrl, y, x] = chunk else {
            warnings.push("the file ends in the middle of a stitch".to_string());
            break;
        };
        let kind = ctrl & KIND;
        if kind == END {
            ended = true;
            break;
        }
        b.count()?;
        let (dx, dy) = decode([ctrl, y, x]);
        match kind {
            STITCH => b.stitch(dx, dy),
            JUMP => b.jump(dx, dy),
            k if FAST_SLOW.contains(&k) => {
                if k & JUMP != 0 {
                    b.jump(dx, dy);
                } else {
                    b.stitch(dx, dy);
                }
            }
            TRIM | TRIM_T1 => {
                b.trim();
                b.travel(dx, dy);
            }
            STOP => b.stop(dx, dy),
            k if (NEEDLE_BASE + 1..=NEEDLE_BASE + MAX_NEEDLE).contains(&k) => b.needle(k - NEEDLE_BASE, dx, dy)?,
            _ => {
                warnings.push("an unknown code ended the file early".to_string());
                break;
            }
        }
    }
    if !ended && warnings.is_empty() {
        warnings.push("the file has no end record".to_string());
    }
    let (mut plan, needles) = b.finish_with_needles(String::new());
    for (block, n) in plan.blocks.iter_mut().zip(&needles) {
        if let Some(n) = n {
            block.thread = default_thread(usize::from(*n) - 1);
        }
    }
    Ok(Input { plan, needles, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Start;
    use proptest::prelude::*;
    use vectorcraft_stitch::{Block, Pt, Step, Thread};

    fn tiny() -> StitchPlan {
        StitchPlan {
            name: "TEST".into(),
            blocks: vec![Block {
                thread: Thread::default(),
                steps: vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 10, y: 0 }), Step::Stitch(Pt { x: 10, y: 10 })],
            }],
        }
    }

    fn first_only() -> WriteOptions {
        WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() }
    }

    #[test]
    fn a_tiny_design_has_exact_bytes() {
        let out = write(&tiny(), &first_only()).unwrap();
        let b = out.bytes;
        assert_eq!(&b[..0x80], [b'0'; 0x80]);
        // Extents: left 0, bottom -10 (y up), right 10, top 0; then 0, 5 records, end (10, -10).
        assert_eq!(&b[0x80..0x94], [0, 0, 0xF6, 0xFF, 10, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 10, 0, 0xF6, 0xFF]);
        assert!(b[0x94..0x100].iter().all(|c| *c == 0));
        // Needle 1, a stitch in place, right 10, down 10, end.
        assert_eq!(&b[0x100..], [0xE9, 0, 0, 0xE0, 0, 0, 0xC0, 0, 10, 0xE0, 10, 0, 0xF8, 0, 0]);
        assert_eq!(out.records, 5);
    }

    #[test]
    fn signs_and_control_codes() {
        assert_eq!(encode(STITCH, -3, 4), Some([0xA0, 4, 3]));
        assert_eq!(encode(JUMP, 3, -4), Some([0xC1, 4, 3]));
        assert_eq!(encode(STITCH, 5, 5), Some([0x80, 5, 5]));
        assert_eq!(encode(TRIM, 0, 0), Some([0xE7, 0, 0]));
        assert_eq!(encode(STOP, 0, 0), Some([0xE8, 0, 0]));
        assert_eq!(encode(NEEDLE_BASE + 15, 0, 0), Some([0xF7, 0, 0]));
        assert_eq!(encode(STITCH, 256, 0), None);
        assert_eq!(encode(STITCH, i32::MIN, 0), None);
        assert_eq!(decode([0xE1, 5, 7]), (-7, -5));
        assert_eq!(decode([0x81, 5, 7]), (7, 5));
    }

    #[test]
    fn needles_follow_the_options_and_stops_have_their_code() {
        let mut p = tiny();
        p.blocks[0].steps.extend([Step::Trim, Step::Stop, Step::Stitch(Pt { x: 10, y: 20 })]);
        p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 20, y: 20 })] });
        p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 30, y: 20 })] });
        let o = WriteOptions { needles: vec![4, 9, 4], ..first_only() };
        let out = write(&p, &o).unwrap();
        let recs: Vec<&[u8]> = out.bytes[0x100..].chunks(3).collect();
        assert_eq!(recs[0], [0xEC, 0, 0], "needle 4 first");
        assert_eq!(recs[4], [0xE7, 0, 0], "trim");
        assert_eq!(recs[5], [0xE8, 0, 0], "stop");
        assert_eq!(recs[7], [0xF1, 0, 0], "needle 9");
        assert_eq!(recs[9], [0xEC, 0, 0], "needle 4 again");
        let back = read(&out.bytes, &ReadOptions::default()).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.needles, vec![Some(4), Some(9), Some(4)]);
        assert_eq!(back.plan.blocks[0].steps.iter().filter(|s| **s == Step::Stop).count(), 1);
        assert_eq!(back.plan.blocks[0].thread, back.plan.blocks[2].thread, "the same needle, the same colour");
        // Without needles in the options, each block takes the next needle, wrapping after 15.
        let o = WriteOptions { needles: vec![0, 16], ..first_only() };
        let recs = write(&p, &o).unwrap().bytes;
        assert_eq!((recs[0x100], recs[0x100 + 7 * 3], recs[0x100 + 9 * 3]), (0xE9, 0xEA, 0xEB));
    }

    #[test]
    fn odd_headers_and_truncated_files_are_survived() {
        assert_eq!(read(&[0u8; 100], &ReadOptions::default()).unwrap_err(), "not a U01 file: the header is incomplete");
        let mut b = write(&tiny(), &first_only()).unwrap().bytes;
        b.truncate(b.len() - 3);
        b.push(0x80);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count(), 3);
        assert!(i.warnings.contains(&"the file ends in the middle of a stitch".to_string()));
        let i = read(&[0xFFu8; 256], &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["the file has no end record".to_string()]);
        // An unknown code (0x19) stops the file with a warning.
        let mut b = vec![b'0'; 256];
        b.extend([0xE0, 0, 0, 0xF9, 0, 0, 0xE0, 1, 1]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["an unknown code ended the file early".to_string()]);
        // Fast and slow records are stitches and jumps; T1 is a trim.
        let mut b = vec![b'0'; 256];
        b.extend([0xE0, 0, 0, 0x82, 0, 5, 0x83, 0, 5, 0x84, 0, 5, 0xE6, 0, 0, 0x85, 0, 5, 0xE0, 0, 0, 0xF8, 0, 0]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert!(i.warnings.is_empty(), "{:?}", i.warnings);
        assert_eq!(
            i.plan.blocks[0].steps,
            vec![
                Step::Stitch(Pt { x: 0, y: 0 }),
                Step::Stitch(Pt { x: 5, y: 0 }),
                Step::Jump(Pt { x: 10, y: 0 }),
                Step::Stitch(Pt { x: 15, y: 0 }),
                Step::Trim,
                Step::Jump(Pt { x: 20, y: 0 }),
                Step::Stitch(Pt { x: 20, y: 0 })
            ]
        );
        assert_eq!(i.needles, vec![None]);
    }

    #[test]
    fn errors_and_limits() {
        let empty = StitchPlan { name: "e".into(), blocks: vec![Block { thread: Thread::default(), steps: vec![Step::Trim] }] };
        assert_eq!(write(&empty, &WriteOptions::default()).err().unwrap(), "nothing to stitch");
        let mut long = tiny();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 10 + 128, y: 10 }));
        assert_eq!(write(&long, &WriteOptions::default()).err().unwrap(), TOO_LONG);
        assert_eq!(read(&vec![0u8; MAX_FILE_BYTES + 1], &ReadOptions::default()).err().unwrap(), "the file is larger than 64 MB");
        let mut b = vec![b'0'; 256];
        for _ in 0..crate::MAX_BLOCKS {
            b.extend([0xE0, 0, 1, 0xEA, 0, 0]);
        }
        assert_eq!(read(&b, &ReadOptions::default()).unwrap_err(), "the file has too many colour changes");
    }

    /// A plan whose moves fit the writer: every move at most 127 per axis, no empty jump, no two
    /// jumps in a row (they read back as one), stops between stitches.
    fn arb_plan() -> impl Strategy<Value = StitchPlan> {
        let op = prop_oneof![
            6 => (-127i32..=127, -127i32..=127).prop_map(|(x, y)| (0u8, x, y)),
            2 => (-127i32..=127, -127i32..=127).prop_filter("a jump that moves", |(x, y)| (*x, *y) != (0, 0)).prop_map(|(x, y)| (1u8, x, y)),
            2 => Just((2u8, 0, 0)),
            1 => Just((3u8, 0, 0)),
        ];
        let block = (prop::collection::vec(op, 0..40), -127i32..=127, -127i32..=127);
        (prop::collection::vec(block, 1..5), -500i32..500, -500i32..500).prop_map(|(blocks, x0, y0)| {
            let mut at = Pt { x: x0, y: y0 };
            let mut out = Vec::new();
            for (i, (ops, fx, fy)) in blocks.into_iter().enumerate() {
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
                        2 => steps.push(Step::Trim),
                        _ if matches!(steps.last(), Some(Step::Stitch(_))) => steps.push(Step::Stop),
                        _ => {}
                    }
                }
                out.push(Block { thread: crate::default_thread(i), steps });
            }
            StitchPlan { name: String::new(), blocks: out }
        })
    }

    /// Every step of every block, positions relative to the first stitch.
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
        fn write_then_read_keeps_stitches_jumps_trims_stops_colours_and_needles(
            plan in arb_plan(),
            centre in any::<bool>(),
            needles in prop::collection::vec(1u8..=15, 0..6),
        ) {
            let start = if centre { Start::Center } else { Start::FirstStitch };
            let o = WriteOptions { start, return_to_start: false, needles: needles.clone(), ..Default::default() };
            let out = write(&plan, &o).unwrap();
            let back = read(&out.bytes, &ReadOptions::default()).unwrap();
            prop_assert!(back.warnings.is_empty(), "{:?}", back.warnings);
            let mut got = relative(&back.plan);
            if let Some(first) = got.first_mut().filter(|b| matches!(b.first(), Some(Step::Jump(_)))) {
                first.remove(0);
            }
            prop_assert_eq!(got, relative(&plan));
            let want: Vec<Option<u8>> = (0..plan.blocks.len()).map(|i| Some(needle(&o, i))).collect();
            prop_assert_eq!(back.needles, want);
        }
    }
}
