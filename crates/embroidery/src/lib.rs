//! VectorCraft embroidery machine files: DST and EXP readers and writers.
//!
//! Writers take a [`StitchPlan`] that is already within the format's limits (the caller
//! prepares it for the machine) and return the file's bytes. Readers treat their input as
//! hostile and return what they could read, with warnings.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dst;
pub mod exp;
mod palette;

pub use palette::default_thread;

use vectorcraft_stitch::{Block, MAX_STEPS, Pt, Step, StitchPlan, Thread};

/// The largest file a reader accepts.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// The most thread blocks (colour changes plus one) a reader builds.
pub const MAX_BLOCKS: usize = 10_000;

/// Where the file's origin is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Start {
    /// The centre of the stitch extents.
    #[default]
    Center,
    /// The first needle point.
    FirstStitch,
}

/// Options for the writers.
#[derive(Clone, Debug)]
pub struct WriteOptions {
    /// Where the file's origin is.
    pub start: Start,
    /// Whether the file ends with travel back to the origin.
    pub return_to_start: bool,
    /// How many jump records the machine reads as a trim (DST only).
    pub jumps_per_trim: u8,
    /// Whether a trim is written before each colour change.
    pub trim_before_color_change: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions { start: Start::Center, return_to_start: true, jumps_per_trim: 3, trim_before_color_change: false }
    }
}

/// Options for the readers.
#[derive(Clone, Debug)]
pub struct ReadOptions {
    /// How many jump records in a row are read as a trim (DST only).
    pub jumps_per_trim: u8,
}

impl Default for ReadOptions {
    fn default() -> Self {
        ReadOptions { jumps_per_trim: 3 }
    }
}

/// A written file.
#[derive(Debug)]
pub struct Output {
    pub bytes: Vec<u8>,
    /// The number of records in the file (for DST, the header's `ST` value: the end record
    /// included).
    pub records: usize,
    pub warnings: Vec<String>,
}

/// A read file.
#[derive(Debug)]
pub struct Input {
    pub plan: StitchPlan,
    pub warnings: Vec<String>,
}

const TOO_LONG: &str = "a stitch is longer than the format allows: prepare the plan first";
const TOO_MANY: &str = "the design has more than 2000000 stitches";

/// What a format writes for each kind of record. Moves are in file orientation (y up); a move
/// beyond the format's limit is an error, never a dropped or shortened record.
trait Sink {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String>;
    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String>;
    fn trim(&mut self) -> Result<(), String>;
    fn color_change(&mut self) -> Result<(), String>;
}

/// The origin of the file, in plan units: see [`Start`]. `None` when the plan has no stitch.
fn origin(plan: &StitchPlan, start: Start) -> Option<Pt> {
    let mut stitches = plan.blocks.iter().flat_map(|b| b.steps.iter()).filter_map(|s| match s {
        Step::Stitch(p) => Some(*p),
        _ => None,
    });
    let first = stitches.next()?;
    match start {
        Start::FirstStitch => Some(first),
        Start::Center => {
            let (mut lo, mut hi) = (first, first);
            for p in stitches {
                lo = Pt { x: lo.x.min(p.x), y: lo.y.min(p.y) };
                hi = Pt { x: hi.x.max(p.x), y: hi.y.max(p.y) };
            }
            // The midpoint of two i32 values lies between them, so it fits an i32.
            let mid = |a: i32, b: i32| i32::try_from((i64::from(a) + i64::from(b)) / 2).unwrap_or(a);
            Some(Pt { x: mid(lo.x, hi.x), y: mid(lo.y, hi.y) })
        }
    }
}

/// The move from `a` to `b` in file orientation (y up), if each axis is within `max`.
fn delta(a: Pt, b: Pt, max: i32) -> Option<(i32, i32)> {
    let dx = i64::from(b.x) - i64::from(a.x);
    let dy = i64::from(a.y) - i64::from(b.y);
    let max = i64::from(max);
    if dx.abs() > max || dy.abs() > max {
        return None;
    }
    Some((i32::try_from(dx).ok()?, i32::try_from(dy).ok()?))
}

/// Travel from `a` to `b` as jumps split into equal parts of at most `max` per axis.
fn travel(sink: &mut impl Sink, a: Pt, b: Pt, max: i32) -> Result<(), String> {
    let (dx, dy) = (i64::from(b.x) - i64::from(a.x), i64::from(b.y) - i64::from(a.y));
    let m = i64::from(max.max(1));
    let parts = ((dx.abs() + m - 1) / m).max((dy.abs() + m - 1) / m);
    // Guards against a far-away design turning into an unbounded number of records.
    if parts > MAX_STEPS as i64 {
        return Err(TOO_MANY.to_string());
    }
    let mut at = a;
    for k in 1..=parts {
        let next = Pt {
            x: i32::try_from(i64::from(a.x) + dx * k / parts).map_err(|_| TOO_LONG.to_string())?,
            y: i32::try_from(i64::from(a.y) + dy * k / parts).map_err(|_| TOO_LONG.to_string())?,
        };
        let (jx, jy) = delta(at, next, max).ok_or(TOO_LONG)?;
        sink.jump(jx, jy)?;
        at = next;
    }
    Ok(())
}

/// Walks the plan in sewing order and hands each record to `sink`, with moves of at most
/// `max` units per axis.
fn walk(plan: &StitchPlan, o: &WriteOptions, max: i32, sink: &mut impl Sink) -> Result<(), String> {
    plan.check()?;
    let start = origin(plan, o.start).ok_or("nothing to stitch")?;
    let mut at = start;
    let mut started = false;
    // Whether stitches were written since the last trim or colour change: the thread is still
    // attached to the work.
    let mut attached = false;
    let color_change = |sink: &mut dyn Sink| -> Result<(), String> {
        if o.trim_before_color_change {
            sink.trim()?;
        }
        sink.color_change()
    };
    for (i, block) in plan.blocks.iter().enumerate() {
        if i > 0 {
            color_change(sink)?;
            attached = false;
        }
        for step in &block.steps {
            match *step {
                Step::Stitch(p) => {
                    if !started {
                        // The leading travel from the origin replaces any jumps before it.
                        travel(sink, at, p, max)?;
                        at = p;
                        started = true;
                    }
                    let (dx, dy) = delta(at, p, max).ok_or(TOO_LONG)?;
                    sink.stitch(dx, dy)?;
                    at = p;
                    attached = true;
                }
                Step::Jump(p) => {
                    if started {
                        let (dx, dy) = delta(at, p, max).ok_or(TOO_LONG)?;
                        sink.jump(dx, dy)?;
                        at = p;
                    }
                }
                Step::Trim => {
                    sink.trim()?;
                    attached = false;
                }
                // A stop pauses the machine; the thread stays attached.
                Step::Stop => color_change(sink)?,
            }
        }
    }
    if o.return_to_start && at != start {
        // Cut first, so the travel back does not drag the thread across the work.
        if attached {
            sink.trim()?;
        }
        travel(sink, at, start, max)?;
    }
    Ok(())
}

/// Builds a plan from records read from a file, with moves in file orientation.
struct Builder {
    blocks: Vec<Block>,
    threads: Vec<Thread>,
    /// The current position, in plan orientation (y down).
    at: Pt,
    /// The position where the current run of jumps began, and its length.
    run: Option<(Pt, usize)>,
    /// A run of this many jumps or more is read as a trim; `None` never.
    jumps_per_trim: Option<usize>,
    records: usize,
}

impl Builder {
    fn new(threads: Vec<Thread>, jumps_per_trim: Option<usize>) -> Builder {
        let first = Block { thread: threads.first().cloned().unwrap_or_else(|| default_thread(0)), steps: Vec::new() };
        Builder { blocks: vec![first], threads, at: Pt::default(), run: None, jumps_per_trim, records: 0 }
    }

    /// Counts one record and refuses a file with too many.
    fn count(&mut self) -> Result<(), String> {
        self.records += 1;
        if self.records > MAX_STEPS {
            return Err(TOO_MANY.to_string());
        }
        Ok(())
    }

    fn push(&mut self, s: Step) {
        if let Some(b) = self.blocks.last_mut() {
            b.steps.push(s);
        }
    }

    fn step(&mut self, dx: i32, dy: i32) {
        self.at = Pt { x: self.at.x.saturating_add(dx), y: self.at.y.saturating_sub(dy) };
    }

    /// Ends the current run of jumps, if any.
    fn flush(&mut self) {
        let Some((from, n)) = self.run.take() else { return };
        if self.jumps_per_trim.is_some_and(|t| n >= t) {
            self.push(Step::Trim);
        }
        if self.at != from {
            self.push(Step::Jump(self.at));
        }
    }

    fn stitch(&mut self, dx: i32, dy: i32) {
        self.flush();
        self.step(dx, dy);
        self.push(Step::Stitch(self.at));
    }

    fn jump(&mut self, dx: i32, dy: i32) {
        let (from, n) = self.run.unwrap_or((self.at, 0));
        self.run = Some((from, n + 1));
        self.step(dx, dy);
    }

    fn trim(&mut self) {
        self.flush();
        self.push(Step::Trim);
    }

    /// A move outside any run of jumps (it never reads as part of a trim): one jump, if it
    /// moves at all.
    fn travel(&mut self, dx: i32, dy: i32) {
        self.flush();
        if (dx, dy) != (0, 0) {
            self.step(dx, dy);
            self.push(Step::Jump(self.at));
        }
    }

    /// Starts the next block, then applies the record's move, if any, as a jump. A file with
    /// more than [`MAX_BLOCKS`] blocks is refused before they are built.
    fn color_change(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.flush();
        let i = self.blocks.len();
        if i >= MAX_BLOCKS {
            return Err("the file has too many colour changes".to_string());
        }
        let thread = self.threads.get(i).cloned().unwrap_or_else(|| default_thread(i));
        self.blocks.push(Block { thread, steps: Vec::new() });
        self.travel(dx, dy);
        Ok(())
    }

    fn finish(mut self, name: String) -> StitchPlan {
        self.flush();
        StitchPlan { name, blocks: self.blocks }
    }
}
