//! DST machine files: a 512-byte header, then three-byte records.

mod header;
mod record;

use crate::{Builder, Input, MAX_FILE_BYTES, Output, ReadOptions, Sink, TOO_LONG, WriteOptions, walk};
use header::Extents;
use record::{COLOR_CHANGE, END, JUMP, MAX_MOVE, STITCH};
use vectorcraft_stitch::StitchPlan;

const JUMPS_PER_TRIM: &str = "jumpsPerTrim must be 2 or more";

/// Whether `bytes` start like a DST file.
pub fn is_dst(bytes: &[u8]) -> bool {
    bytes.starts_with(b"LA:")
}

/// Records as they are written, with the position and extents they reach.
struct Records {
    out: Vec<[u8; 3]>,
    jumps_per_trim: u8,
    at: (i64, i64),
    ext: Extents,
    color_changes: usize,
}

impl Records {
    /// Adds a record. `extend`: the position it reaches counts toward the header's extents (a
    /// stitch or real travel, not the small jumps of a trim).
    fn push(&mut self, dx: i32, dy: i32, flags: u8, extend: bool) -> Result<(), String> {
        let r = record::encode(dx, dy, flags).ok_or(TOO_LONG)?;
        self.out.push(r);
        self.at = (self.at.0 + i64::from(dx), self.at.1 + i64::from(dy));
        if extend {
            let e = &mut self.ext;
            e.max_x = e.max_x.max(self.at.0);
            e.min_x = e.min_x.max(-self.at.0);
            e.max_y = e.max_y.max(self.at.1);
            e.min_y = e.min_y.max(-self.at.1);
        }
        Ok(())
    }
}

impl Sink for Records {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.push(dx, dy, STITCH, true)
    }

    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.push(dx, dy, JUMP, true)
    }

    /// Small jumps that sum to zero, alternating 2 units right and left of the trim position
    /// and ending on it.
    fn trim(&mut self) -> Result<(), String> {
        let n = i32::from(self.jumps_per_trim);
        let mut offset = 0;
        for k in 0..n {
            let next = if k == n - 1 {
                0
            } else if k % 2 == 0 {
                2
            } else {
                -2
            };
            self.push(next - offset, 0, JUMP, false)?;
            offset = next;
        }
        Ok(())
    }

    fn color_change(&mut self) -> Result<(), String> {
        self.out.push(COLOR_CHANGE);
        self.color_changes += 1;
        Ok(())
    }
}

/// The DST file for `plan`, which must already fit the format (see [`crate::WriteOptions`]).
pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    if o.jumps_per_trim < 2 {
        return Err(JUMPS_PER_TRIM.to_string());
    }
    let mut r = Records { out: Vec::new(), jumps_per_trim: o.jumps_per_trim, at: (0, 0), ext: Extents::default(), color_changes: 0 };
    walk(plan, o, MAX_MOVE, &mut r)?;
    r.out.push(END);
    let end = (i32::try_from(r.at.0).unwrap_or(i32::MAX), i32::try_from(r.at.1).unwrap_or(i32::MAX));
    let head = header::write(&plan.name, r.out.len(), r.color_changes, r.ext, end);
    let mut bytes = Vec::with_capacity(header::LEN + 3 * r.out.len());
    bytes.extend_from_slice(&head);
    bytes.extend(r.out.iter().flatten());
    Ok(Output { bytes, records: r.out.len(), warnings: header::overflow(r.out.len(), r.color_changes) })
}

/// The stitch plan in a DST file, y down, with warnings for what could not be read.
pub fn read(bytes: &[u8], o: &ReadOptions) -> Result<Input, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is larger than 64 MB".to_string());
    }
    if o.jumps_per_trim < 2 {
        return Err(JUMPS_PER_TRIM.to_string());
    }
    let (Some(head), Some(body)) = (bytes.get(..header::LEN), bytes.get(header::LEN..)) else {
        return Err("not a DST file: the header is incomplete".to_string());
    };
    let fields = header::read(head);
    let mut b = Builder::new(fields.threads, Some(usize::from(o.jumps_per_trim)));
    let mut warnings = Vec::new();
    let mut ended = false;
    let mut sequins = false;
    for chunk in body.chunks(3) {
        let &[b0, b1, b2] = chunk else {
            warnings.push("the file ends in the middle of a stitch".to_string());
            break;
        };
        if b2 & 0xF3 == 0xF3 {
            ended = true;
            break;
        }
        b.count()?;
        let (dx, dy) = record::decode([b0, b1, b2]);
        match b2 & 0xC0 {
            0xC0 => b.color_change(dx, dy)?,
            0x80 => b.jump(dx, dy),
            0x40 => {
                // A sequin record: no step of its own, but its move still counts, so the
                // rest of the design stays in place.
                b.travel(dx, dy);
                if !sequins {
                    warnings.push("sequin records were skipped".to_string());
                    sequins = true;
                }
            }
            _ => b.stitch(dx, dy),
        }
    }
    if !ended {
        warnings.push("the file has no end record".to_string());
    }
    Ok(Input { plan: b.finish(fields.name), warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReadOptions, Start, WriteOptions};
    use proptest::prelude::*;
    use vectorcraft_stitch::{Block, Pt, Step, StitchPlan, Thread};

    fn tiny() -> StitchPlan {
        StitchPlan {
            name: "TEST".into(),
            blocks: vec![Block {
                thread: Thread::default(),
                steps: vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 10, y: 0 }), Step::Stitch(Pt { x: 10, y: 10 })],
            }],
        }
    }

    #[test]
    fn a_tiny_design_has_exact_bytes() {
        let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() };
        let b = write(&tiny(), &o).unwrap().bytes;
        let head = format!(
            "LA:{:<16}\rST:{:>7}\rCO:{:>3}\r+X:{:>5}\r-X:{:>5}\r+Y:{:>5}\r-Y:{:>5}\rAX:+{:>5}\rAY:-{:>5}\rMX:+{:>5}\rMY:+{:>5}\rPD:******\r\x1a",
            "TEST", 4, 0, 10, 0, 0, 10, 10, 10, 0, 0
        );
        assert_eq!(&b[..head.len()], head.as_bytes());
        assert_eq!(head.len(), 125);
        assert!(b[125..512].iter().all(|c| *c == 0x20));
        assert_eq!(&b[512..], [0x00, 0x00, 0x03, 0x05, 0x00, 0x03, 0x50, 0x00, 0x03, 0x00, 0x00, 0xF3]);
    }

    #[test]
    fn the_name_is_sanitised_to_sixteen_ascii_characters() {
        let mut p = tiny();
        p.name = "狗 portrait.final-version-2".into();
        let b = write(&p, &WriteOptions::default()).unwrap().bytes;
        assert_eq!(&b[..20], b"LA: portraitfinal-v\r");
        assert_eq!(&b[20..23], b"ST:");
    }

    #[test]
    fn trims_colour_changes_and_centre_start() {
        let mut p = tiny();
        p.blocks[0].steps.push(Step::Trim);
        p.blocks.push(Block {
            thread: Thread { rgb: [255, 0, 0], ..Default::default() },
            steps: vec![Step::Jump(Pt { x: 10, y: 0 }), Step::Stitch(Pt { x: 10, y: 0 }), Step::Stitch(Pt { x: 0, y: 0 })],
        });
        let b = write(&p, &WriteOptions::default()).unwrap().bytes;
        let recs: Vec<&[u8]> = b[512..].chunks(3).collect();
        assert_eq!(recs[0], [0x69, 0x41, 0x83]); // from the centre (5, 5) to the first stitch: a jump of (-5, +5) in file orientation
        assert!(recs.contains(&[0x00u8, 0x00, 0xC3].as_slice()));
        assert_eq!(recs.last().unwrap(), &[0x00u8, 0x00, 0xF3].as_slice());
        assert_eq!(&b[31..37], b"CO:  1");
        let back = read(&b, &ReadOptions::default()).unwrap().plan;
        assert_eq!(back.blocks.len(), 2);
        assert_eq!(back.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1);
    }

    #[test]
    fn odd_headers_and_truncated_files_are_survived() {
        assert_eq!(read(&[0u8; 100], &ReadOptions::default()).unwrap_err(), "not a DST file: the header is incomplete");
        let mut b = write(&tiny(), &WriteOptions::default()).unwrap().bytes;
        b[23..30].copy_from_slice(b"0000004"); // zero-padded ST
        b.truncate(b.len() - 3); // no end record
        b.push(0x01); // a partial record
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count(), 3);
        assert!(i.warnings.contains(&"the file has no end record".to_string()));
        assert!(i.warnings.contains(&"the file ends in the middle of a stitch".to_string()));
    }

    #[test]
    fn errors_and_warnings_have_their_messages() {
        let empty = StitchPlan { name: "e".into(), blocks: vec![Block { thread: Thread::default(), steps: vec![Step::Trim] }] };
        assert_eq!(write(&empty, &WriteOptions::default()).err().unwrap(), "nothing to stitch");
        let mut long = tiny();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 132, y: 10 }));
        assert_eq!(write(&long, &WriteOptions::default()).err().unwrap(), "a stitch is longer than the format allows: prepare the plan first");
        let o = WriteOptions { jumps_per_trim: 1, ..Default::default() };
        assert_eq!(write(&tiny(), &o).err().unwrap(), "jumpsPerTrim must be 2 or more");
        assert_eq!(read(&vec![0u8; MAX_FILE_BYTES + 1], &ReadOptions::default()).err().unwrap(), "the file is larger than 64 MB");
        // A sequin record between two stitches is skipped, with one warning.
        let mut b = write(&tiny(), &WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() }).unwrap().bytes;
        b.splice(515..515, [0x00, 0x00, 0x43, 0x00, 0x00, 0x43]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["sequin records were skipped".to_string()]);
        assert_eq!(i.plan.blocks[0].steps.len(), 3);
    }

    #[test]
    fn trim_jumps_are_small_and_return_to_the_trim_position() {
        let mut p = tiny();
        p.blocks[0].steps.push(Step::Trim);
        let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, jumps_per_trim: 4, ..Default::default() };
        let b = write(&p, &o).unwrap().bytes;
        let recs: Vec<&[u8]> = b[512..].chunks(3).collect();
        let moves: Vec<(i32, i32)> = recs[3..7].iter().map(|r| record::decode([r[0], r[1], r[2]])).collect();
        assert_eq!(moves, vec![(2, 0), (-4, 0), (4, 0), (-2, 0)]);
        assert!(recs[3..7].iter().all(|r| r[2] == 0x83));
    }

    #[test]
    fn the_thread_is_trimmed_before_the_return_to_the_start() {
        // Centre (5, 5): the last stitch (10, 10) is attached, so the travel back is trimmed first.
        let b = write(&tiny(), &WriteOptions::default()).unwrap().bytes;
        let recs: Vec<[u8; 3]> = b[512..].chunks(3).map(|r| [r[0], r[1], r[2]]).collect();
        let n = recs.len();
        assert_eq!(n, 1 + 3 + 3 + 1 + 1, "{recs:?}");
        let moves: Vec<(i32, i32)> = recs[4..8].iter().map(|r| record::decode(*r)).collect();
        assert_eq!(moves, vec![(2, 0), (-4, 0), (2, 0), (-5, 5)]);
        assert!(recs[4..8].iter().all(|r| r[2] == 0x83));
        assert_eq!(recs[n - 1], END);
        assert_eq!(&b[20..30], b"ST:      9");
        assert_eq!(write(&tiny(), &WriteOptions::default()).unwrap().records, 9);
        // A plan that already ends with a trim gets no second one.
        let mut p = tiny();
        p.blocks[0].steps.push(Step::Trim);
        let b = write(&p, &WriteOptions::default()).unwrap().bytes;
        assert_eq!((b.len() - 512) / 3, 9);
        // Without the return, the file ends on the last stitch.
        let b = write(&tiny(), &WriteOptions { return_to_start: false, ..Default::default() }).unwrap().bytes;
        assert_eq!((b.len() - 512) / 3, 5);
    }

    #[test]
    fn header_extents_leave_out_the_trim_jumps() {
        let mut p = tiny();
        p.blocks[0].steps.push(Step::Trim);
        let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() };
        let b = write(&p, &o).unwrap().bytes;
        // The trim's jumps reach x = 12, but the design is 10 units wide.
        assert_eq!(&b[38..47], b"+X:   10\r");
        assert_eq!(&b[47..56], b"-X:    0\r");
    }

    #[test]
    fn a_move_out_of_range_is_an_error_not_a_lost_record() {
        let mut r = Records { out: Vec::new(), jumps_per_trim: 3, at: (0, 0), ext: Extents::default(), color_changes: 0 };
        assert_eq!(r.stitch(122, 0).unwrap_err(), "a stitch is longer than the format allows: prepare the plan first");
        assert_eq!(r.jump(0, -122).unwrap_err(), "a stitch is longer than the format allows: prepare the plan first");
        assert!(r.out.is_empty());
        assert_eq!(r.at, (0, 0));
    }

    #[test]
    fn counts_too_large_for_the_header_are_reported() {
        let mut p = tiny();
        for _ in 0..1000 {
            p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 0, y: 0 })] });
        }
        let out = write(&p, &WriteOptions::default()).unwrap();
        assert_eq!(&out.bytes[31..37], b"CO:999");
        assert_eq!(out.warnings, vec!["the colour change count is too large for the file header".to_string()]);
        assert!(write(&tiny(), &WriteOptions::default()).unwrap().warnings.is_empty());
        assert_eq!(header::overflow(10_000_000, 0), vec!["the stitch count is too large for the file header".to_string()]);
        assert!(header::overflow(9_999_999, 999).is_empty());
    }

    /// A header from a written file, followed by `body`.
    fn with_body(body: &[[u8; 3]]) -> Vec<u8> {
        let mut b = write(&tiny(), &WriteOptions::default()).unwrap().bytes;
        b.truncate(512);
        b.extend(body.iter().flatten());
        b
    }

    #[test]
    fn a_colour_change_with_a_move_applies_the_move() {
        let cc = record::encode(5, 0, 0xC0).unwrap();
        let b = with_body(&[record::encode(0, 0, STITCH).unwrap(), cc, record::encode(1, 0, STITCH).unwrap(), END]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks.len(), 2);
        assert_eq!(i.plan.blocks[1].steps, vec![Step::Jump(Pt { x: 5, y: 0 }), Step::Stitch(Pt { x: 6, y: 0 })]);
        assert!(i.warnings.is_empty(), "{:?}", i.warnings);
    }

    #[test]
    fn any_third_byte_with_the_end_bits_ends_the_file() {
        // 0xFB carries every bit of 0xF3 and one more.
        let b = with_body(&[record::encode(0, 0, STITCH).unwrap(), [0, 0, 0xFB], record::encode(1, 0, STITCH).unwrap()]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].steps, vec![Step::Stitch(Pt { x: 0, y: 0 })]);
        assert!(i.warnings.is_empty(), "{:?}", i.warnings);
    }

    #[test]
    fn a_file_of_too_many_colour_changes_is_refused_while_reading() {
        let mut body = vec![COLOR_CHANGE; crate::MAX_BLOCKS];
        body.push(END);
        assert_eq!(read(&with_body(&body), &ReadOptions::default()).unwrap_err(), "the file has too many colour changes");
        body.remove(0);
        assert_eq!(read(&with_body(&body), &ReadOptions::default()).unwrap().plan.blocks.len(), crate::MAX_BLOCKS);
    }

    #[test]
    fn header_threads_colour_the_blocks() {
        let mut p = tiny();
        p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 0, y: 0 })] });
        let mut b = write(&p, &WriteOptions::default()).unwrap().bytes;
        // An extended field in place of the 0x1A that ends the header's fields.
        let tc = b"TC:#102030,Navy,N1\r\x1a";
        b[124..124 + tc.len()].copy_from_slice(tc);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.name, "TEST");
        assert_eq!(i.plan.blocks[0].thread, Thread { rgb: [0x10, 0x20, 0x30], name: Some("Navy".into()), code: Some("N1".into()) });
        assert_eq!(i.plan.blocks[1].thread, crate::default_thread(1));
        assert!(is_dst(&b));
        assert!(!is_dst(b"XX:"));
    }

    /// A plan whose moves fit the format: every move at most 121 units per axis, and no
    /// run of three or more jumps without a trim before it.
    fn arb_plan() -> impl Strategy<Value = StitchPlan> {
        let op = prop_oneof![
            6 => (-121i32..=121, -121i32..=121).prop_map(|(x, y)| (0u8, x, y)),
            2 => (-121i32..=121, -121i32..=121).prop_map(|(x, y)| (1u8, x, y)),
            1 => Just((2u8, 0, 0)),
        ];
        let block = (prop::collection::vec(op, 0..40), -121i32..=121, -121i32..=121);
        (prop::collection::vec(block, 1..4), -500i32..500, -500i32..500).prop_map(|(blocks, x0, y0)| {
            let mut at = Pt { x: x0, y: y0 };
            let mut out = Vec::new();
            for (i, (ops, fx, fy)) in blocks.into_iter().enumerate() {
                at = Pt { x: at.x + fx, y: at.y + fy };
                let mut steps = vec![Step::Stitch(at)];
                let mut jumps = 0;
                for (kind, dx, dy) in ops {
                    match kind {
                        0 => {
                            at = Pt { x: at.x + dx, y: at.y + dy };
                            steps.push(Step::Stitch(at));
                            jumps = 0;
                        }
                        1 if jumps < 2 => {
                            at = Pt { x: at.x + dx, y: at.y + dy };
                            steps.push(Step::Jump(at));
                            jumps += 1;
                        }
                        1 => {}
                        _ => {
                            steps.push(Step::Trim);
                            jumps = 0;
                        }
                    }
                }
                out.push(Block { thread: crate::default_thread(i), steps });
            }
            StitchPlan { name: "P".into(), blocks: out }
        })
    }

    fn stitches(p: &StitchPlan) -> Vec<Vec<Pt>> {
        let first = p.blocks.iter().flat_map(|b| b.steps.iter()).find_map(|s| if let Step::Stitch(q) = s { Some(*q) } else { None });
        let f = first.unwrap_or_default();
        p.blocks
            .iter()
            .map(|b| b.steps.iter().filter_map(|s| if let Step::Stitch(q) = s { Some(Pt { x: q.x - f.x, y: q.y - f.y }) } else { None }).collect())
            .collect()
    }

    proptest! {
        #[test]
        fn write_then_read_keeps_the_stitches(plan in arb_plan(), centre in any::<bool>(), back in any::<bool>()) {
            let start = if centre { Start::Center } else { Start::FirstStitch };
            let o = WriteOptions { start, return_to_start: back, ..Default::default() };
            let out = write(&plan, &o).unwrap();
            let read_back = read(&out.bytes, &ReadOptions::default()).unwrap();
            prop_assert!(read_back.warnings.is_empty());
            prop_assert_eq!(stitches(&read_back.plan), stitches(&plan));
        }
    }
}
