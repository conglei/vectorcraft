//! TBF machine files (Tajima): a 1,536-byte header of DST-style fields with the needle order
//! and thread colours, then three-byte records `[dx, dy, control]`. See
//! `docs/superpowers/specs/2026-11-13-embroidery-tbf-format.md`.
//!
//! TBF has no signature of its own: a file is recognised by its `.tbf` extension.

use crate::dst::header::{Extents, fit, label, signed};
use crate::{Builder, Input, MAX_FILE_BYTES, MAX_NEEDLE, Output, ReadOptions, Sink, TOO_LONG, WriteOptions, default_thread, needle, walk};
use vectorcraft_stitch::{StitchPlan, Thread};

/// The header's length.
const HEADER: usize = 0x600;
/// Where the fields start (after the version), the name, the needle order and the colours.
const FIELDS: usize = 0x80;
const NAME_AT: usize = 0x83;
const NAME_WIDTH: usize = 16;
const ORDER_AT: usize = 0x10A;
/// The needle order holds one byte per needle change.
const ORDER_LEN: usize = 0x100;
const COLOURS_AT: usize = 0x20E;
/// Where the colour list must end: the header's `\r` and 0x1A follow.
const COLOURS_END: usize = 0x376;
/// One colour entry: `E`, red, green, blue, a space.
const COLOUR: u8 = 0x45;
/// The largest move per axis: a signed byte.
const MAX_MOVE: i32 = 127;
const STITCH: u8 = 0x80;
const JUMP: u8 = 0x90;
const NEEDLE: u8 = 0x81;
const STOP: u8 = 0x40;
const TRIM: u8 = 0x86;
const END: u8 = 0x8F;
/// The byte after the end record.
const TERMINAL: u8 = 0x1A;

/// A signed move as one byte; a move beyond +-127 is an error.
fn byte(v: i32) -> Result<u8, String> {
    if v.abs() > MAX_MOVE {
        return Err(TOO_LONG.to_string());
    }
    let [b] = i8::try_from(v).map_err(|_| TOO_LONG.to_string())?.to_le_bytes();
    Ok(b)
}

fn signed8(v: u8) -> i32 {
    i32::from(i8::from_le_bytes([v]))
}

/// Records as they are written, with the position and extents they reach (y up, from the origin).
struct Records<'a> {
    o: &'a WriteOptions,
    out: Vec<[u8; 3]>,
    at: (i64, i64),
    ext: Extents,
    /// The block being written.
    block: usize,
    /// The needle of each needle change, in order: the header's needle order.
    order: Vec<u8>,
    /// Stitches were written since the last trim: the thread is attached.
    attached: bool,
}

impl Records<'_> {
    fn push(&mut self, dx: i32, dy: i32, ctrl: u8) -> Result<(), String> {
        let (x, y) = (byte(dx)?, byte(dy)?);
        self.out.push([x, y, ctrl]);
        self.at = (self.at.0 + i64::from(dx), self.at.1 + i64::from(dy));
        let e = &mut self.ext;
        e.max_x = e.max_x.max(self.at.0);
        e.min_x = e.min_x.max(-self.at.0);
        e.max_y = e.max_y.max(self.at.1);
        e.min_y = e.min_y.max(-self.at.1);
        Ok(())
    }

    fn needle(&mut self) -> Result<(), String> {
        if self.order.len() >= ORDER_LEN {
            return Err(format!("a TBF file holds at most {ORDER_LEN} needle changes"));
        }
        self.order.push(needle(self.o, self.block));
        self.push(0, 0, NEEDLE)
    }
}

impl Sink for Records<'_> {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.attached = true;
        self.push(dx, dy, STITCH)
    }

    /// A jump that does not move is left out: readers take it for a trim.
    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        if (dx, dy) == (0, 0) {
            return Ok(());
        }
        self.push(dx, dy, JUMP)
    }

    fn trim(&mut self) -> Result<(), String> {
        self.attached = false;
        self.push(0, 0, TRIM)
    }

    /// A needle change, after a trim when the thread is still attached: TBF writes the cut
    /// explicitly.
    fn color_change(&mut self) -> Result<(), String> {
        if self.attached {
            self.trim()?;
        }
        self.block += 1;
        self.needle()
    }

    /// The file sets the first block's needle before anything else.
    fn begin(&mut self) -> Result<(), String> {
        self.needle()
    }

    fn stop(&mut self) -> Option<Result<(), String>> {
        Some(self.push(0, 0, STOP))
    }
}

/// The colour of each needle the plan uses, needle 1 first: the colour of the first block sewn
/// with it; a needle the plan skips takes the built-in colour of its number.
fn needle_colours(plan: &StitchPlan, o: &WriteOptions) -> Vec<[u8; 3]> {
    let mut colours: Vec<Option<[u8; 3]>> = Vec::new();
    for (i, b) in plan.blocks.iter().enumerate() {
        let n = usize::from(needle(o, i));
        if colours.len() < n {
            colours.resize(n, None);
        }
        if let Some(c) = colours.get_mut(n - 1).filter(|c| c.is_none()) {
            *c = Some(b.thread.rgb);
        }
    }
    colours.into_iter().enumerate().map(|(i, c)| c.unwrap_or_else(|| default_thread(i).rgb)).collect()
}

/// The header: the version `3.00`, spaces to 0x80, then the DST fields `LA` to `AY`, `TP` and
/// `JC`, the needle order `DO` (one byte per needle change, 256 bytes) and the colours `DA`
/// (`E`, red, green, blue, space per needle), spaces to 0x376, `\r`, 0x1A, spaces to 0x600.
fn header(name: &str, r: &Records, records: usize, colours: &[[u8; 3]]) -> Vec<u8> {
    let mut h = b"3.00".to_vec();
    h.resize(FIELDS, b' ');
    let records = fit(i64::try_from(records).unwrap_or(i64::MAX), 7);
    let changes = fit(i64::try_from(r.order.len()).unwrap_or(i64::MAX), 3);
    let (ax, ay) = r.at;
    let e = r.ext;
    let text = format!(
        "LA:{:<16}\rST:{records:>7}\rCO:{changes:>3}\r+X:{:>5}\r-X:{:>5}\r+Y:{:>5}\r-Y:{:>5}\rAX:{}\rAY:{}\rTP:{:<32}\rJC:3\rDO:",
        label(name),
        fit(e.max_x, 5),
        fit(e.min_x, 5),
        fit(e.max_y, 5),
        fit(e.min_y, 5),
        signed(ax),
        signed(ay),
        "EG/",
    );
    h.extend(text.bytes());
    let mut order = r.order.clone();
    order.resize(ORDER_LEN, 0);
    h.extend(order);
    h.extend(b"\rDA:");
    for c in colours.iter().take((COLOURS_END - COLOURS_AT) / 5) {
        h.extend([COLOUR, c[0], c[1], c[2], b' ']);
    }
    h.resize(COLOURS_END, b' ');
    h.extend([b'\r', TERMINAL]);
    h.resize(HEADER, b' ');
    h
}

/// The TBF file for `plan`, which must already fit the format (see [`crate::WriteOptions`];
/// `jumps_per_trim` does not apply: a trim has its own code).
pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String> {
    let mut r = Records { o, out: Vec::new(), at: (0, 0), ext: Extents::default(), block: 0, order: Vec::new(), attached: false };
    walk(plan, o, MAX_MOVE, &mut r)?;
    r.out.push([0, 0, END]);
    let records = r.out.len();
    let mut bytes = header(&plan.name, &r, records, &needle_colours(plan, o));
    bytes.extend(r.out.iter().flatten());
    bytes.push(TERMINAL);
    let mut warnings = Vec::new();
    if records > 9_999_999 {
        warnings.push("the stitch count is too large for the file header".to_string());
    }
    Ok(Output { bytes, records, warnings })
}

/// The name and thread colours (one per needle, needle 1 first) and the needle order of a header.
fn read_header(h: &[u8]) -> (String, Vec<Thread>, Vec<u8>) {
    let name: String = h
        .get(NAME_AT..NAME_AT + NAME_WIDTH)
        .unwrap_or_default()
        .iter()
        .take_while(|c| **c != b'\r')
        .map(|c| if c.is_ascii_graphic() || *c == b' ' { char::from(*c) } else { ' ' })
        .collect::<String>()
        .trim()
        .to_string();
    let order =
        if h.get(ORDER_AT - 3..ORDER_AT) == Some(b"DO:") { h.get(ORDER_AT..ORDER_AT + ORDER_LEN).unwrap_or_default().to_vec() } else { Vec::new() };
    let mut threads = Vec::new();
    if h.get(COLOURS_AT - 3..COLOURS_AT) == Some(b"DA:") {
        for c in h.get(COLOURS_AT..COLOURS_END).unwrap_or_default().chunks(5) {
            let &[COLOUR, r, g, b, _] = c else { break };
            threads.push(Thread { rgb: [r, g, b], name: None, code: None, brand: None });
        }
    }
    (name, threads, order)
}

/// The stitch plan in a TBF file, y down, with warnings for what could not be read. A needle
/// change after stitches, and a stop, start a new block; the needle of each block (from the
/// header's needle order) is kept in [`Input::needles`], and its colour is that needle's in the
/// header's colour list, else the built-in one of its number.
pub fn read(bytes: &[u8], _o: &ReadOptions) -> Result<Input, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is larger than 64 MB".to_string());
    }
    let (Some(head), Some(body)) = (bytes.get(..HEADER), bytes.get(HEADER..)) else {
        return Err("not a TBF file: the header is incomplete".to_string());
    };
    let (name, threads, order) = read_header(head);
    let mut b = Builder::new(Vec::new(), None);
    let mut warnings = Vec::new();
    let mut ended = false;
    let mut changes = 0usize;
    for chunk in body.chunks(3) {
        let &[x, y, ctrl] = chunk else {
            warnings.push("the file ends in the middle of a stitch".to_string());
            break;
        };
        if ctrl == END {
            ended = true;
            break;
        }
        b.count()?;
        let (dx, dy) = (signed8(x), signed8(y));
        match ctrl {
            STITCH => b.stitch(dx, dy),
            JUMP if (dx, dy) == (0, 0) => b.trim(),
            JUMP => b.jump(dx, dy),
            TRIM => b.trim(),
            STOP => b.stop(0, 0),
            NEEDLE => {
                // A needle the order does not name takes the next in turn; 0 is a stop.
                let n = order.get(changes).copied().unwrap_or_else(|| u8::try_from(changes % usize::from(MAX_NEEDLE)).unwrap_or(0) + 1);
                changes += 1;
                match n {
                    0 => b.stop(0, 0),
                    n if n <= MAX_NEEDLE => b.needle(n, 0, 0)?,
                    _ => {
                        warnings.push("a needle change names a needle above 15".to_string());
                        break;
                    }
                }
            }
            _ => {
                warnings.push("an unknown code ended the file early".to_string());
                break;
            }
        }
    }
    if !ended && warnings.is_empty() {
        warnings.push("the file has no end record".to_string());
    }
    let (mut plan, needles) = b.finish_with_needles(name);
    for (i, (block, n)) in plan.blocks.iter_mut().zip(&needles).enumerate() {
        let k = n.map_or(i, |n| usize::from(n) - 1);
        block.thread = threads.get(k).cloned().unwrap_or_else(|| default_thread(k));
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
                thread: Thread { rgb: [255, 0, 0], ..Thread::default() },
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
        let b = &out.bytes;
        assert_eq!(b.len(), HEADER + 5 * 3 + 1);
        assert_eq!(&b[..4], b"3.00");
        assert!(b[4..0x80].iter().all(|c| *c == b' '));
        let fields = String::from_utf8_lossy(&b[0x80..ORDER_AT]).to_string();
        assert_eq!(
            fields,
            "LA:TEST            \rST:      5\rCO:  1\r+X:   10\r-X:    0\r+Y:    0\r-Y:   10\rAX:+   10\rAY:-   10\rTP:EG/                             \rJC:3\rDO:"
        );
        assert_eq!(b[ORDER_AT], 1);
        assert!(b[ORDER_AT + 1..ORDER_AT + ORDER_LEN].iter().all(|c| *c == 0));
        assert_eq!(&b[ORDER_AT + ORDER_LEN..COLOURS_AT + 5], b"\rDA:E\xff\x00\x00 ");
        assert_eq!(&b[COLOURS_END..COLOURS_END + 2], b"\r\x1a");
        assert!(b[COLOURS_END + 2..HEADER].iter().all(|c| *c == b' '));
        // Needle 1, a stitch in place, right 10, down 10, end, then 0x1A.
        assert_eq!(&b[HEADER..], [0, 0, 0x81, 0, 0, 0x80, 10, 0, 0x80, 0, 0xF6, 0x80, 0, 0, 0x8F, 0x1A]);
        assert_eq!(out.records, 5);
    }

    #[test]
    fn needle_changes_follow_a_trim_and_name_their_needles() {
        let mut p = tiny();
        p.blocks[0].steps.extend([Step::Stop, Step::Jump(Pt { x: 30, y: 10 }), Step::Stitch(Pt { x: 30, y: 10 })]);
        p.blocks.push(Block { thread: Thread { rgb: [0, 0, 255], ..Thread::default() }, steps: vec![Step::Stitch(Pt { x: 20, y: 20 })] });
        p.blocks.push(Block { thread: Thread { rgb: [255, 0, 0], ..Thread::default() }, steps: vec![Step::Stitch(Pt { x: 30, y: 20 })] });
        let o = WriteOptions { needles: vec![3, 1, 3], ..first_only() };
        let out = write(&p, &o).unwrap();
        let recs: Vec<&[u8]> = out.bytes[HEADER..].chunks(3).collect();
        assert_eq!(recs[4], [0, 0, 0x40], "stop");
        assert_eq!(recs[5], [20, 0, 0x90], "jump");
        assert_eq!((recs[7], recs[8]), (&[0u8, 0, 0x86][..], &[0u8, 0, 0x81][..]), "a trim, then the needle change");
        assert_eq!(&out.bytes[ORDER_AT..ORDER_AT + 4], [3, 1, 3, 0]);
        assert_eq!(&out.bytes[31 + 0x80..37 + 0x80], b"CO:  3");
        // Needle 1 blue, needle 2 unused (built-in), needle 3 red.
        assert_eq!(&out.bytes[COLOURS_AT..COLOURS_AT + 15], b"E\x00\x00\xff E\xc8\x1e\x28 E\xff\x00\x00 ");
        let back = read(&out.bytes, &ReadOptions::default()).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.plan.name, "TEST");
        assert_eq!(back.needles, vec![Some(3), Some(1), Some(3)]);
        let rgb: Vec<[u8; 3]> = back.plan.blocks.iter().map(|b| b.thread.rgb).collect();
        assert_eq!(rgb, vec![[255, 0, 0], [0, 0, 255], [255, 0, 0]]);
        assert_eq!(back.plan.blocks[0].steps.iter().filter(|s| **s == Step::Stop).count(), 1);
    }

    #[test]
    fn odd_files_are_survived() {
        assert_eq!(read(&[0u8; 100], &ReadOptions::default()).unwrap_err(), "not a TBF file: the header is incomplete");
        let i = read(&[0xFFu8; HEADER], &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["the file has no end record".to_string()]);
        let mut b = vec![b' '; HEADER];
        // stitch, jump of nothing (a trim), stitch, unknown.
        b.extend([0, 0, 0x80, 0, 0, 0x90, 5, 5, 0x80, 0, 0, 0x55, 1, 1, 0x80]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.warnings, vec!["an unknown code ended the file early".to_string()]);
        assert_eq!(i.plan.blocks[0].steps, vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Trim, Step::Stitch(Pt { x: 5, y: -5 })]);
        // Needle changes without a needle order take the next needle; a truncated record warns.
        let mut b = vec![b' '; HEADER];
        b.extend([0, 0, 0x81, 0, 0, 0x80, 0, 0, 0x81, 1, 0, 0x80, 7]);
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.needles, vec![Some(1), Some(2)]);
        assert_eq!(i.warnings, vec!["the file ends in the middle of a stitch".to_string()]);
        let mut b = write(&tiny(), &first_only()).unwrap().bytes;
        b[ORDER_AT] = 16;
        assert_eq!(read(&b, &ReadOptions::default()).unwrap().warnings, vec!["a needle change names a needle above 15".to_string()]);
        b[ORDER_AT] = 0;
        let i = read(&b, &ReadOptions::default()).unwrap();
        assert_eq!(i.plan.blocks[0].steps.first(), Some(&Step::Stop), "needle 0 is a stop");
    }

    #[test]
    fn errors_and_limits() {
        let mut long = tiny();
        long.blocks[0].steps.push(Step::Stitch(Pt { x: 10 + 128, y: 10 }));
        assert_eq!(write(&long, &WriteOptions::default()).err().unwrap(), TOO_LONG);
        assert_eq!(read(&vec![0u8; MAX_FILE_BYTES + 1], &ReadOptions::default()).err().unwrap(), "the file is larger than 64 MB");
        let mut many = tiny();
        for i in 0..ORDER_LEN {
            many.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt { x: i as i32 % 50, y: 0 })] });
        }
        assert_eq!(write(&many, &first_only()).err().unwrap(), "a TBF file holds at most 256 needle changes");
        let mut b = vec![b' '; HEADER];
        for _ in 0..crate::MAX_BLOCKS {
            b.extend([0, 1, 0x80, 0, 0, 0x81]);
        }
        assert_eq!(read(&b, &ReadOptions::default()).unwrap_err(), "the file has too many colour changes");
    }

    /// A plan whose moves fit the format: every move at most 127 per axis, no empty jump, no two
    /// jumps in a row, no trim right before a block's end (TBF adds one there itself).
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
            StitchPlan { name: "P".into(), blocks: out }
        })
    }

    /// Every step of every block, positions relative to the first stitch, with the trim TBF
    /// writes before a needle change (the last step of a block but the last, after stitches)
    /// left out.
    fn relative(p: &StitchPlan, drop_trims: bool) -> Vec<Vec<Step>> {
        let f = p.blocks.iter().flat_map(|b| b.steps.iter()).find_map(|s| if let Step::Stitch(q) = s { Some(*q) } else { None }).unwrap_or_default();
        let shift = |q: Pt| Pt { x: q.x - f.x, y: q.y - f.y };
        let n = p.blocks.len();
        p.blocks
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let mut steps: Vec<Step> = b
                    .steps
                    .iter()
                    .map(|s| match *s {
                        Step::Stitch(q) => Step::Stitch(shift(q)),
                        Step::Jump(q) => Step::Jump(shift(q)),
                        other => other,
                    })
                    .collect();
                if drop_trims && i + 1 < n && steps.last() == Some(&Step::Trim) {
                    steps.pop();
                }
                steps
            })
            .collect()
    }

    proptest! {
        #[test]
        fn write_then_read_keeps_stitches_jumps_stops_colours_and_needles(
            plan in arb_plan(),
            centre in any::<bool>(),
            needles in prop::collection::vec(1u8..=15, 0..6),
        ) {
            let start = if centre { Start::Center } else { Start::FirstStitch };
            let o = WriteOptions { start, return_to_start: false, needles, ..Default::default() };
            let out = write(&plan, &o).unwrap();
            let back = read(&out.bytes, &ReadOptions::default()).unwrap();
            prop_assert!(back.warnings.is_empty(), "{:?}", back.warnings);
            let mut got = relative(&back.plan, true);
            if let Some(first) = got.first_mut().filter(|b| matches!(b.first(), Some(Step::Jump(_)))) {
                first.remove(0);
            }
            prop_assert_eq!(got, relative(&plan, true));
            let want: Vec<Option<u8>> = (0..plan.blocks.len()).map(|i| Some(needle(&o, i))).collect();
            prop_assert_eq!(back.needles, want);
        }
    }
}
