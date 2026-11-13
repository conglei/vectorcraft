//! VectorCraft embroidery machine files: DST, DSB, EXP, U01, TBF and PES/PEC readers and writers.
//!
//! Writers take a [`StitchPlan`] that is already within the format's limits (the caller
//! prepares it for the machine) and return the file's bytes. Readers treat their input as
//! hostile and return what they could read, with warnings.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod dsb;
pub mod dst;
pub mod exp;
mod palette;
pub mod pes;
pub mod tbf;
pub mod u01;

pub use palette::default_thread;

use vectorcraft_stitch::{Block, MAX_STEPS, Pt, Step, StitchPlan, TOO_MANY_STEPS, Thread};

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
    /// A point the design names (Embroidery › Sequence: the design start), in plan units.
    Point(Pt),
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
    /// The needle (1–15) each block is sewn with, for the formats that name needles (U01, TBF).
    /// A block past the end of the list, or given 0 or a needle over 15, takes the next needle in
    /// turn: block `i` is needle `i % 15 + 1`, as DSB writes.
    pub needles: Vec<u8>,
}

impl Default for WriteOptions {
    fn default() -> Self {
        WriteOptions { start: Start::Center, return_to_start: true, jumps_per_trim: 3, trim_before_color_change: false, needles: Vec::new() }
    }
}

/// The most needles a format's needle codes name.
pub const MAX_NEEDLE: u8 = 15;

/// The needle block `block` is sewn with: see [`WriteOptions::needles`].
fn needle(o: &WriteOptions, block: usize) -> u8 {
    match o.needles.get(block) {
        Some(&n) if (1..=MAX_NEEDLE).contains(&n) => n,
        // `block % 15` is below 15, so it fits a u8.
        _ => u8::try_from(block % usize::from(MAX_NEEDLE)).unwrap_or(0) + 1,
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
    /// The needle each block of `plan` names, for the formats that name needles (U01, TBF);
    /// empty for the others. `None`: the file set no needle for that block.
    pub needles: Vec<Option<u8>>,
    pub warnings: Vec<String>,
}

const TOO_LONG: &str = "a stitch is longer than the format allows: prepare the plan first";

/// What a format writes for each kind of record. Moves are in file orientation (y up); a move
/// beyond the format's limit is an error, never a dropped or shortened record.
trait Sink {
    fn stitch(&mut self, dx: i32, dy: i32) -> Result<(), String>;
    fn jump(&mut self, dx: i32, dy: i32) -> Result<(), String>;
    fn trim(&mut self) -> Result<(), String>;
    /// The change to the next block's thread.
    fn color_change(&mut self) -> Result<(), String>;
    /// Called once before the first record, for a format that sets the first needle.
    fn begin(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// A stop in a format with a stop code of its own; `None` when it has none, and a stop is
    /// written as a colour change.
    fn stop(&mut self) -> Option<Result<(), String>> {
        None
    }
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
        Start::Point(p) => Some(p),
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
        return Err(TOO_MANY_STEPS.to_string());
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
    sink.begin()?;
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
                Step::Stop => match sink.stop() {
                    Some(r) => r?,
                    None => color_change(sink)?,
                },
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

/// The shortest piece, on its larger axis, of a long stitch split into jumps: half the shortest
/// `maxStitch` (1 mm). Equal moves shorter than this are not read as a split stitch.
const MIN_PIECE: i64 = 5;

/// A run of jump records being read.
#[derive(Clone, Copy)]
struct Run {
    /// Where it began.
    from: Pt,
    /// How many records it has.
    n: usize,
    /// The smallest and largest move on each axis: `[min dx, max dx, min dy, max dy]`.
    moves: [i64; 4],
    /// The shortest move, on its larger axis.
    shortest: i64,
}

impl Run {
    fn new(from: Pt, dx: i32, dy: i32) -> Run {
        let (dx, dy) = (i64::from(dx), i64::from(dy));
        Run { from, n: 1, moves: [dx, dx, dy, dy], shortest: dx.abs().max(dy.abs()) }
    }

    fn add(mut self, dx: i32, dy: i32) -> Run {
        let (dx, dy) = (i64::from(dx), i64::from(dy));
        let [x0, x1, y0, y1] = self.moves;
        self.moves = [x0.min(dx), x1.max(dx), y0.min(dy), y1.max(dy)];
        self.shortest = self.shortest.min(dx.abs().max(dy.abs()));
        self
    }

    /// Whether the run and the stitch `(dx, dy)` after it are one long stitch split into equal
    /// pieces: every move within one unit of the others on each axis, and none shorter than
    /// [`MIN_PIECE`]. The writers split long stitches this way (see `vectorcraft_stitch::prepare`),
    /// as other software does; a trim's jumps (small moves that cancel) and travel that ends in a
    /// stitch in place or a shorter stitch are not.
    fn is_split_stitch(self, dx: i32, dy: i32) -> bool {
        let r = self.add(dx, dy);
        let [x0, x1, y0, y1] = r.moves;
        x1 - x0 <= 1 && y1 - y0 <= 1 && r.shortest >= MIN_PIECE
    }
}

/// Builds a plan from records read from a file, with moves in file orientation.
struct Builder {
    blocks: Vec<Block>,
    /// The needle each block names, if any: one per block.
    needles: Vec<Option<u8>>,
    threads: Vec<Thread>,
    /// The current position, in plan orientation (y down).
    at: Pt,
    /// The current run of jumps.
    run: Option<Run>,
    /// A run of this many jumps or more is read as a trim, unless it is a split stitch
    /// ([`Run::is_split_stitch`]); `None` never.
    jumps_per_trim: Option<usize>,
    records: usize,
}

impl Builder {
    fn new(threads: Vec<Thread>, jumps_per_trim: Option<usize>) -> Builder {
        let first = Block { thread: threads.first().cloned().unwrap_or_else(|| default_thread(0)), steps: Vec::new() };
        Builder { blocks: vec![first], needles: vec![None], threads, at: Pt::default(), run: None, jumps_per_trim, records: 0 }
    }

    /// Counts one record and refuses a file with too many.
    fn count(&mut self) -> Result<(), String> {
        self.records += 1;
        if self.records > MAX_STEPS {
            return Err(TOO_MANY_STEPS.to_string());
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

    /// Ends the current run of jumps, if any. `stitch`: the move of the stitch that ends it.
    fn flush(&mut self, stitch: Option<(i32, i32)>) {
        let Some(run) = self.run.take() else { return };
        let split = stitch.is_some_and(|(dx, dy)| run.is_split_stitch(dx, dy));
        if self.jumps_per_trim.is_some_and(|t| run.n >= t) && !split {
            self.push(Step::Trim);
        }
        if self.at != run.from {
            self.push(Step::Jump(self.at));
        }
    }

    fn stitch(&mut self, dx: i32, dy: i32) {
        self.flush(Some((dx, dy)));
        self.step(dx, dy);
        self.push(Step::Stitch(self.at));
    }

    fn jump(&mut self, dx: i32, dy: i32) {
        self.run = Some(match self.run {
            Some(r) => Run { n: r.n.saturating_add(1), ..r.add(dx, dy) },
            None => Run::new(self.at, dx, dy),
        });
        self.step(dx, dy);
    }

    fn trim(&mut self) {
        self.flush(None);
        self.push(Step::Trim);
    }

    /// A move outside any run of jumps (it never reads as part of a trim): one jump, if it
    /// moves at all.
    fn travel(&mut self, dx: i32, dy: i32) {
        self.flush(None);
        if (dx, dy) != (0, 0) {
            self.step(dx, dy);
            self.push(Step::Jump(self.at));
        }
    }

    /// Starts the next block, then applies the record's move, if any, as a jump. A file with
    /// more than [`MAX_BLOCKS`] blocks is refused before they are built.
    fn color_change(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        self.flush(None);
        let i = self.blocks.len();
        if i >= MAX_BLOCKS {
            return Err("the file has too many colour changes".to_string());
        }
        let thread = self.threads.get(i).cloned().unwrap_or_else(|| default_thread(i));
        self.blocks.push(Block { thread, steps: Vec::new() });
        self.needles.push(None);
        self.travel(dx, dy);
        Ok(())
    }

    /// A change to needle `n`, then the record's move, if any, as a jump. It names the needle of
    /// the current block while that block has no stitch yet (the needle a file sets before its
    /// first stitch, or after a stop), and starts the next block otherwise.
    fn needle(&mut self, n: u8, dx: i32, dy: i32) -> Result<(), String> {
        self.flush(None);
        let stitched = self.blocks.last().is_some_and(|b| b.steps.iter().any(|s| matches!(s, Step::Stitch(_))));
        if stitched {
            self.color_change(0, 0)?;
        }
        if let Some(last) = self.needles.last_mut() {
            *last = Some(n);
        }
        self.travel(dx, dy);
        Ok(())
    }

    /// A stop (the machine pauses, the thread stays), then the record's move, if any, as a jump.
    fn stop(&mut self, dx: i32, dy: i32) {
        self.flush(None);
        self.push(Step::Stop);
        self.travel(dx, dy);
    }

    fn finish(self, name: String) -> StitchPlan {
        self.finish_with_needles(name).0
    }

    /// The plan, and the needle each block names.
    fn finish_with_needles(mut self, name: String) -> (StitchPlan, Vec<Option<u8>>) {
        self.flush(None);
        (StitchPlan { name, blocks: self.blocks }, self.needles)
    }
}
