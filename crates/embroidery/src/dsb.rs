//! DSB machine files (Barudan): a 512-byte DST-style header, then three-byte records
//! `[control, |dy|, |dx|]`. See `docs/superpowers/specs/2026-11-11-embroidery-dsb-format.md`.
//!
//! DSB has no signature: a file is recognised by its `.dsb` extension, as EXP is.

use crate::dst::header::{self, Extents};
use crate::{Builder, Input, MAX_FILE_BYTES, Output, ReadOptions, Sink, TOO_LONG, WriteOptions, walk};
use vectorcraft_stitch::StitchPlan;

/// The largest move per axis: one byte.
const MAX_MOVE: i32 = 255;
/// Control byte of a stitch; `| X_NEG` / `| Y_NEG` for a move left or down (file orientation).
const STITCH: u8 = 0x00;
/// Control byte of a jump (a move without a stitch).
const JUMP: u8 = 0x01;
const X_NEG: u8 = 0x20;
const Y_NEG: u8 = 0x40;
/// Cut the thread.
const TRIM: u8 = 0xE7;
/// Stop for a thread change.
const STOP: u8 = 0xE8;
/// A change to needle 1..=15 is `NEEDLE_BASE + n`.
const NEEDLE_BASE: u8 = 0xE8;
const NEEDLES: usize = 15;
/// The end of the file.
const END: u8 = 0xF8;
/// Other control bytes some files end with.
const END_ALT: [u8; 2] = [0x91, 0x87];

/// The record for a move of (`dx`, `dy`), y up. `None` beyond +-255.
fn encode(dx: i32, dy: i32, base: u8) -> Option<[u8; 3]> {
    let x = u8::try_from(dx.checked_abs()?).ok()?;
    let y = u8::try_from(dy.checked_abs()?).ok()?;
    let ctrl = base | if dx < 0 { X_NEG } else { 0 } | if dy < 0 { Y_NEG } else { 0 };
    Some([ctrl, y, x])
}

/// The move a record makes, y up.
fn decode(r: [u8; 3]) -> (i32, i32) {
    let [ctrl, y, x] = r;
    let (x, y) = (i32::from(x), i32::from(y));
    (if ctrl & X_NEG != 0 { -x } else { x }, if ctrl & Y_NEG != 0 { -y } else { y })
}

/// Records as they are written, with the position and extents they reach.
struct Records {
    out: Vec<[u8; 3]>,
    at: (i64, i64),
    ext: Extents,
    color_changes: usize,
}

impl Records {
    fn new() -> Records {
        Records { out: Vec::new(), at: (0, 0), ext: Extents::default(), color_changes: 0 }
    }

    fn push(&mut self, dx: i32, dy: i32, base: u8) -> Result<(), String> {
        let r = encode(dx, dy, base).ok_or(TOO_LONG)?;
        self.out.push(r);
        self.at = (self.at.0 + i64::from(dx), self.at.1 + i64::from(dy));
        let e = &mut self.ext;
        e.max_x = e.max_x.max(self.at.0);
        e.min_x = e.min_x.max(-self.at.0);
        e.max_y = e.max_y.max(self.at.1);
        e.min_y = e.min_y.max(-self.at.1);
        Ok(())
    }
}

impl Sink for Records {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.push(dx, dy, STITCH)
    }

    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.push(dx, dy, JUMP)
    }

    fn trim(&mut self) -> Result<(), String> {
        self.out.push([TRIM, 0, 0]);
        Ok(())
    }

    /// A change to the next needle, wrapping after 15.
    fn color_change(&mut self) -> Result<(), String> {
        self.color_changes += 1;
        let needle = self.color_changes % NEEDLES + 1;
        let code = u8::try_from(usize::from(NEEDLE_BASE) + needle).map_err(|_| TOO_LONG.to_string())?;
        self.out.push([code, 0, 0]);
        Ok(())
    }
}

/// The DSB file for `plan`, which must already fit the format (see [`crate::WriteOptions`];
/// `jumps_per_trim` does not apply: a trim has its own code).
pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    let mut r = Records::new();
    walk(plan, o, MAX_MOVE, &mut r)?;
    r.out.push([END, 0, 0]);
    let end = (i32::try_from(r.at.0).unwrap_or(i32::MAX), i32::try_from(r.at.1).unwrap_or(i32::MAX));
    let head = header::write(&plan.name, r.out.len(), r.color_changes, r.ext, end);
    let mut bytes = Vec::with_capacity(header::LEN + 3 * r.out.len());
    bytes.extend_from_slice(&head);
    bytes.extend(r.out.iter().flatten());
    Ok(Output { bytes, records: r.out.len(), warnings: header::overflow(r.out.len(), r.color_changes) })
}

/// The stitch plan in a DSB file, y down, with warnings for what could not be read. Every
/// stop or needle change starts a new block.
pub fn read(bytes: &[u8], _o: &ReadOptions) -> Result<Input, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is larger than 64 MB".to_string());
    }
    let (Some(head), Some(body)) = (bytes.get(..header::LEN), bytes.get(header::LEN..)) else {
        return Err("not a DSB file: the header is incomplete".to_string());
    };
    let fields = header::read(head);
    let mut b = Builder::new(fields.threads, None);
    let mut warnings = Vec::new();
    let mut ended = false;
    for chunk in body.chunks(3) {
        let &[ctrl, y, x] = chunk else {
            warnings.push("the file ends in the middle of a stitch".to_string());
            break;
        };
        if ctrl == END || END_ALT.contains(&ctrl) {
            ended = true;
            break;
        }
        b.count()?;
        let (dx, dy) = decode([ctrl, y, x]);
        match ctrl {
            TRIM => b.trim(),
            STOP..=0xF7 => b.color_change(0, 0)?,
            c if c & 0x1F == STITCH => b.stitch(dx, dy),
            c if c & 0x1F == JUMP => b.jump(dx, dy),
            _ => {
                warnings.push("an unknown code ended the file early".to_string());
                break;
            }
        }
    }
    if !ended && warnings.is_empty() {
        warnings.push("the file has no end record".to_string());
    }
    Ok(Input { plan: b.finish(fields.name), warnings })
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
        let b = write(&tiny(), &first_only()).unwrap().bytes;
        assert_eq!(&b[..3], b"LA:");
        assert_eq!(&b[20..30], b"ST:      4");
        assert_eq!(&b[512..], [0x00, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x40, 0x0A, 0x00, 0xF8, 0x00, 0x00]);
    }

    #[test]
    fn signs_jumps_trims_and_needle_changes_have_their_codes() {
        assert_eq!(encode(-3, 4, STITCH), Some([0x20, 4, 3]));
        assert_eq!(encode(3, -4, JUMP), Some([0x41, 4, 3]));
        assert_eq!(encode(-255, -255, JUMP), Some([0x61, 255, 255]));
        assert_eq!(encode(256, 0, STITCH), None);
        assert_eq!(encode(i32::MIN, 0, STITCH), None);
        assert_eq!(decode([0x61, 5, 7]), (-7, -5));
        let mut p = tiny();
        p.blocks[0].steps.push(Step::Trim);
        p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 20, y: 10 })] });
        p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 20, y: 10 })] });
        let out = write(&p, &first_only()).unwrap();
        let recs: Vec<&[u8]> = out.bytes[512..].chunks(3).collect();
        assert_eq!(recs[3], [0xE7, 0, 0]);
        assert_eq!(recs[4], [0xEA, 0, 0]); // needle 2
        assert_eq!(recs[6], [0xEB, 0, 0]); // needle 3
        assert_eq!(&out.bytes[31..37], b"CO:  2");
        assert_eq!(read(&out.bytes, &ReadOptions::default()).unwrap().plan.blocks.len(), 3);
    }

    #[test]
    fn needles_wrap_after_fifteen() {
        let mut r = Records::new();
        for _ in 0..16 {
            r.color_change().unwrap();
        }
        assert_eq!(r.out[13][0], 0xE8 + 15);
        assert_eq!(r.out[14][0], 0xE8 + 1);
        assert_eq!(r.out[15][0], 0xE8 + 2);
    }

    #[test]
    fn odd_headers_and_truncated_files_are_survived() {
        assert_eq!(read(&[0u8; 100], &ReadOptions::default()).unwrap_err(), "not a DSB file: the header is incomplete");
        let mut b = write(&tiny(), &first_only()).unwrap().bytes;
        b.truncate(b.len() - 3); // no end record
        b.push(0x01); // a partial record
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count(), 3);
        assert!(i.warnings.contains(&"the file ends in the middle of a stitch".to_string()));
        // A garbage header is fine; so is an empty body.
        let i = read(&[0xFFu8; 512], &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["the file has no end record".to_string()]);
        // An unknown control byte stops the file with a warning.
        let mut b = vec![b' '; 512];
        b.extend([0x00, 0, 0, 0x55, 0, 0, 0x00, 1, 1]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["an unknown code ended the file early".to_string()]);
        assert_eq!(i.plan.blocks[0].steps.len(), 1);
        // The alternative end codes end the file.
        for end in END_ALT {
            let mut b = vec![b' '; 512];
            b.extend([0x00, 0, 0, end, 0, 0, 0x00, 1, 1]);
            let i = read(&b, &ReadOptions::default()).unwrap();
            assert!(i.warnings.is_empty());
            assert_eq!(i.plan.blocks[0].steps.len(), 1);
        }
    }

    #[test]
    fn errors_have_their_messages() {
        let empty = StitchPlan { name: "e".into(), blocks: vec![Block { thread: Thread::default(), steps: vec![Step::Trim] }] };
        assert_eq!(write(&empty, &WriteOptions::default()).err().unwrap(), "nothing to stitch");
        let mut long = tiny();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 10 + 256, y: 10 }));
        assert_eq!(write(&long, &WriteOptions::default()).err().unwrap(), "a stitch is longer than the format allows: prepare the plan first");
        assert_eq!(read(&vec![0u8; MAX_FILE_BYTES + 1], &ReadOptions::default()).err().unwrap(), "the file is larger than 64 MB");
    }

    #[test]
    fn a_move_out_of_range_is_an_error_not_a_lost_record() {
        let mut r = Records::new();
        assert!(r.stitch(256, 0).is_err());
        assert!(r.jump(0, -256).is_err());
        assert!(r.out.is_empty());
        assert_eq!(r.at, (0, 0));
        assert!(r.stitch(255, -255).is_ok());
    }

    #[test]
    fn a_trim_record_reaches_the_plan_as_a_trim_wherever_it_sits() {
        let mut b = vec![b' '; 512];
        // stitch, stitch, trim, jump, stitch, trim, needle change, stitch, end
        b.extend([0x00, 0, 0, 0x00, 0, 10, 0xE7, 0, 0, 0x01, 0, 100, 0x00, 0, 10, 0xE7, 0, 0, 0xE9, 0, 0, 0x00, 0, 10, 0xF8, 0, 0]);
        let plan = read(&b, &ReadOptions::default()).unwrap().plan;
        assert_eq!(plan.blocks.len(), 2);
        assert_eq!(plan.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 2, "{:?}", plan.blocks[0].steps);
        assert!(plan.blocks[1].steps.iter().all(|s| *s != Step::Trim));
    }

    #[test]
    fn too_many_colour_changes_are_refused_while_reading() {
        let mut b = vec![b' '; 512];
        for _ in 0..crate::MAX_BLOCKS {
            b.extend([0xE9, 0, 0]);
        }
        assert_eq!(read(&b, &ReadOptions::default()).unwrap_err(), "the file has too many colour changes");
    }

    /// A plan whose moves fit the format: every move at most 255 per axis, no empty jump, no two
    /// jumps in a row (they read back as one).
    fn arb_plan() -> impl Strategy<Value = StitchPlan> {
        let op = prop_oneof![
            6 => (-255i32..=255, -255i32..=255).prop_map(|(x, y)| (0u8, x, y)),
            2 => (-255i32..=255, -255i32..=255).prop_filter("a jump that moves", |(x, y)| (*x, *y) != (0, 0)).prop_map(|(x, y)| (1u8, x, y)),
            2 => Just((2u8, 0, 0)),
        ];
        let block = (prop::collection::vec(op, 0..40), -255i32..=255, -255i32..=255);
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
                        _ => steps.push(Step::Trim),
                    }
                }
                out.push(Block { thread: crate::default_thread(i), steps });
            }
            StitchPlan { name: "P".into(), blocks: out }
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
        fn write_then_read_keeps_stitches_jumps_trims_and_colours(plan in arb_plan(), centre in any::<bool>()) {
            let start = if centre { Start::Center } else { Start::FirstStitch };
            let o = WriteOptions { start, return_to_start: false, ..Default::default() };
            let out = write(&plan, &o).unwrap();
            let back = read(&out.bytes, &ReadOptions::default()).unwrap();
            prop_assert!(back.warnings.is_empty(), "{:?}", back.warnings);
            let mut got = relative(&back.plan);
            // From the centre, the file starts with travel to the first stitch.
            if let Some(first) = got.first_mut().filter(|b| matches!(b.first(), Some(Step::Jump(_)))) {
                first.remove(0);
            }
            prop_assert_eq!(got, relative(&plan));
        }
    }
}
