//! EXP machine files: no header, two-byte stitches and four-byte control records.

use crate::{Builder, Input, MAX_FILE_BYTES, Output, ReadOptions, Sink, WriteOptions, walk};
use vectorcraft_stitch::StitchPlan;

/// The largest move per axis.
const MAX_MOVE: i32 = 127;
/// The first byte of a control record.
const CONTROL: u8 = 0x80;
const JUMP: u8 = 0x04;
const COLOR_CHANGE: [u8; 4] = [CONTROL, 0x01, 0x00, 0x00];
const TRIM: [u8; 4] = [CONTROL, 0x80, 0x07, 0x00];

/// A signed move as one byte. Moves reach `Sink` within +-127.
fn byte(v: i32) -> u8 {
    let [b] = i8::try_from(v).unwrap_or(0).to_le_bytes();
    b
}

impl Sink for Vec<u8> {
    fn stitch(&mut self, dx: i32, dy: i32) {
        self.extend([byte(dx), byte(dy)]);
    }

    fn jump(&mut self, dx: i32, dy: i32) {
        self.extend([CONTROL, JUMP, byte(dx), byte(dy)]);
    }

    fn trim(&mut self) {
        self.extend(TRIM);
    }

    fn color_change(&mut self) {
        self.extend(COLOR_CHANGE);
    }
}

/// The EXP file for `plan`, which must already fit the format (see [`crate::WriteOptions`];
/// `jumps_per_trim` does not apply: a trim has its own code).
pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    let mut bytes = Vec::new();
    walk(plan, o, MAX_MOVE, &mut bytes)?;
    Ok(Output { bytes, warnings: Vec::new() })
}

/// The stitch plan in an EXP file, y down, with warnings for what could not be read. Every
/// colour change or stop starts a new block, with a colour from the built-in list.
pub fn read(bytes: &[u8], _o: &ReadOptions) -> Result<Input, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is larger than 64 MB".to_string());
    }
    let mut b = Builder::new(Vec::new(), None);
    let mut warnings = Vec::new();
    let mut rest = bytes;
    let signed = |v: u8| i32::from(i8::from_le_bytes([v]));
    while !rest.is_empty() {
        if rest == [0x1A] {
            break;
        }
        b.count()?;
        let n = match *rest {
            [CONTROL, code, x, y, ..] => {
                let (dx, dy) = (signed(x), signed(y));
                match code {
                    0x02 => b.stitch(dx, dy),
                    0x04 | 0x06 => b.jump(dx, dy),
                    0x10 => break,
                    0x80 => b.trim(),
                    c if c & 0x01 != 0 => b.color_change(dx, dy),
                    _ => {
                        warnings.push("an unknown code ended the file early".to_string());
                        break;
                    }
                }
                4
            }
            [CONTROL, ..] => {
                warnings.push("the file ends in the middle of a stitch".to_string());
                break;
            }
            [x, y, ..] => {
                b.stitch(signed(x), signed(y));
                2
            }
            _ => {
                warnings.push("the file ends in the middle of a stitch".to_string());
                break;
            }
        };
        rest = rest.get(n..).unwrap_or_default();
    }
    Ok(Input { plan: b.finish(String::new()), warnings })
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
        assert_eq!(write(&tiny(), &o).unwrap().bytes, [0x00, 0x00, 0x0A, 0x00, 0x00, 0xF6]);
    }

    #[test]
    fn jumps_trims_and_colour_changes_have_their_codes() {
        let mut p = tiny();
        p.blocks[0].steps.extend([Step::Trim, Step::Jump(Pt { x: 30, y: 10 }), Step::Stitch(Pt { x: 30, y: 10 })]);
        p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: 30, y: 20 })] });
        let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() };
        assert_eq!(
            write(&p, &o).unwrap().bytes,
            [0x00, 0x00, 0x0A, 0x00, 0x00, 0xF6, 0x80, 0x80, 0x07, 0x00, 0x80, 0x04, 0x14, 0x00, 0x00, 0x00, 0x80, 0x01, 0x00, 0x00, 0x00, 0xF6]
        );
    }

    #[test]
    fn the_reader_is_lenient() {
        let bytes = [0x00, 0x00, 0x80, 0x02, 0x05, 0x00, 0x80, 0x06, 0x05, 0x00, 0x00, 0x00, 0x80, 0x10, 0x00, 0x00, 0x1A];
        let i = read(&bytes, &ReadOptions::default()).unwrap();
        assert_eq!(
            i.plan.blocks[0].steps,
            vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 5, y: 0 }), Step::Jump(Pt { x: 10, y: 0 }), Step::Stitch(Pt { x: 10, y: 0 })]
        );
        assert!(i.warnings.is_empty());
        let cut = read(&[0x00, 0x00, 0x80, 0x7E, 0x00, 0x00, 0x05, 0x00], &ReadOptions::default()).unwrap();
        assert_eq!(cut.warnings, vec!["an unknown code ended the file early".to_string()]);
    }

    #[test]
    fn centre_start_travel_trims_and_limits() {
        // Centre (5, 5): a jump of (-5, +5) to the first stitch, and back at the end.
        let b = write(&tiny(), &WriteOptions::default()).unwrap().bytes;
        assert_eq!(&b[..4], [0x80, 0x04, 0xFB, 0x05]);
        assert_eq!(&b[b.len() - 4..], [0x80, 0x04, 0xFB, 0x05]);
        // A trim and stop read back; jump runs become one jump.
        let i = read(
            &[
                0x00, 0x00, 0x80, 0x80, 0x07, 0x00, 0x80, 0x04, 0x7F, 0x00, 0x80, 0x04, 0x7F, 0x00, 0x80, 0x04, 0x7F, 0x00, 0x00, 0x00, 0x80, 0x01,
                0x00, 0x00, 0x01, 0x01,
            ],
            &ReadOptions::default(),
        )
        .unwrap();
        assert_eq!(i.plan.blocks.len(), 2);
        assert_eq!(
            i.plan.blocks[0].steps,
            vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Trim, Step::Jump(Pt { x: 381, y: 0 }), Step::Stitch(Pt { x: 381, y: 0 })]
        );
        assert_eq!(i.plan.blocks[1].steps, vec![Step::Stitch(Pt { x: 382, y: -1 })]);
        assert_eq!(i.plan.blocks[1].thread, crate::default_thread(1));
        // Limits and odd endings.
        let mut long = tiny();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 138, y: 10 }));
        assert_eq!(write(&long, &WriteOptions::default()).err().unwrap(), "a stitch is longer than the format allows: prepare the plan first");
        // The longest move fits, and jumpsPerTrim is ignored.
        long.blocks[0].steps.pop();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 137, y: 10 }));
        assert!(write(&long, &WriteOptions { jumps_per_trim: 0, ..Default::default() }).is_ok());
        assert_eq!(read(&vec![0u8; MAX_FILE_BYTES + 1], &ReadOptions::default()).err().unwrap(), "the file is larger than 64 MB");
        assert_eq!(
            read(&[0x00, 0x00, 0x80, 0x04, 0x01], &ReadOptions::default()).unwrap().warnings,
            vec!["the file ends in the middle of a stitch".to_string()]
        );
        assert_eq!(read(&[0x00, 0x00, 0x1A], &ReadOptions::default()).unwrap().warnings, Vec::<String>::new());
    }

    /// A plan whose moves fit the format: every move at most 127 units per axis.
    fn arb_plan() -> impl Strategy<Value = StitchPlan> {
        let op = prop_oneof![
            6 => (-127i32..=127, -127i32..=127).prop_map(|(x, y)| (0u8, x, y)),
            2 => (-127i32..=127, -127i32..=127).prop_map(|(x, y)| (1u8, x, y)),
            1 => Just((2u8, 0, 0)),
        ];
        let block = (prop::collection::vec(op, 0..40), -127i32..=127, -127i32..=127);
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
