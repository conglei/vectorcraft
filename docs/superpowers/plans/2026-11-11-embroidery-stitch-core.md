# Embroidery Stitch Core and DST/EXP Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stroked paths in a document export to DST and EXP stitch files that a commercial multi-needle machine can run, and DST and EXP files open as documents.

**Architecture:** A new L2 crate `vectorcraft-stitch` holds the stitch plan, machine profiles, plan preparation, and the conversions between documents and plans. A new L3 crate `vectorcraft-embroidery` turns plans into DST and EXP bytes and back, with no document types. The engine adds two format rows, export and open hooks, and two `stitch.*` commands.

**Tech Stack:** Rust (workspace edition), `kurbo`, `serde`/`serde_json`, `proptest` for property tests. No new external dependencies.

**Spec:** `docs/superpowers/specs/2026-11-11-embroidery-stitch-core-design.md`. Read it first; this plan argues from it and does not repeat its tables.

## Global Constraints

- No panics and no `unsafe` outside tests: no `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`, no `v[i]` or `&s[a..b]` on data-derived indices, no integer division by a value that can be zero. Check floats are finite before casting.
- New crates: `[lints] workspace = true`, `license.workspace = true`, `#![forbid(unsafe_code)]` at the crate root, and they must pass `cargo check --target wasm32-unknown-unknown`.
- Clean room: write DST and EXP code and test bytes from the tables in the spec. Do not open any GPL or AGPL source. Do not copy code from any other implementation.
- User-visible strings (command labels, params docs, format labels, warnings) name no vendor or product. `cargo xtask brands` enforces this.
- Units: plan coordinates are `i32` in 0.1 mm, y down. Files are y up. Documents are in points; 1 pt = 25.4/72 mm.
- Limits: at most 2,000,000 steps per plan; reader input at most 64 MB.
- One task per commit, message prefix `E1.<task>:`. Before each commit run the task's tests plus `cargo fmt --all` and `cargo clippy -p <crate> --all-targets -- -D warnings`. The last task runs the full `cargo xtask ci`.
- Do not change anything under `.github/`.

## Review Focus

Each line names an input the spec implies but does not spell out. The test that pins it is in the task named.

1. A closed path must stitch its closing segment back to the first point (Task 5, `closed_path_returns_to_its_start`).
2. A path with one anchor, or zero length, yields no stitches and no empty block, and a document holding only such paths is the "nothing to stitch" error (Task 5, `degenerate_paths_add_nothing`).
3. A design name with non-ASCII characters or more than 16 characters must not shift the DST header fields (Task 3, `the_name_is_sanitised_to_sixteen_ascii_characters`).
4. A DST file shorter than 512 bytes, with zero-padded header numbers, with CRLF after fields, or with no end record must give an error or a readable plan, never a panic (Task 3, `odd_headers_and_truncated_files_are_survived`).
5. `stitchLength` given as NaN, infinity, zero, a negative number or a string is refused with a message naming the allowed range (Task 6, `bad_stitch_options_are_refused`).

---

### Task 1: The `stitch` crate and the plan model

**Files:**
- Create: `crates/stitch/Cargo.toml`, `crates/stitch/src/lib.rs`, `crates/stitch/src/plan.rs`
- Modify: `Cargo.toml` (workspace dependency `vectorcraft-stitch = { path = "crates/stitch" }`, next to `vectorcraft-trace`), `xtask/src/layers.rs` (append `("stitch", Class::Layer(2))` to the "Crates added later" section of `TABLE`)

**Interfaces:**
- Produces, all `pub` and re-exported from `lib.rs`:

```rust
pub const UNITS_PER_MM: f64 = 10.0;
pub const MAX_STEPS: usize = 2_000_000;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)] pub struct Pt { pub x: i32, pub y: i32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Step { Stitch(Pt), Jump(Pt), Trim, Stop }
#[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct Thread { pub rgb: [u8; 3], pub name: Option<String>, pub code: Option<String> }
#[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct Block { pub thread: Thread, pub steps: Vec<Step> }
#[derive(Clone, Debug, Default, PartialEq, Eq)] pub struct StitchPlan { pub name: String, pub blocks: Vec<Block> }
impl Pt { pub fn from_mm(x: f64, y: f64) -> Option<Pt>; pub fn from_pt(x: f64, y: f64) -> Option<Pt>; pub fn to_mm(self) -> (f64, f64); pub fn dist(self, o: Pt) -> f64 /* units */ }
impl StitchPlan { pub fn steps(&self) -> usize; pub fn check(&self) -> Result<(), String>; pub fn points(&self) -> impl Iterator<Item = Pt> + '_ }
```

`from_mm` and `from_pt` round to the nearest unit and return `None` for a non-finite input or one outside ±10,000,000 units. `check` errors with `"the design has more than 2000000 stitches"` past `MAX_STEPS`. `points` yields the position of every `Stitch` and `Jump`, in order.

- [ ] **Step 1: Scaffold the crate.** `Cargo.toml` modelled on `crates/trace/Cargo.toml`: name `vectorcraft-stitch`, description `"VectorCraft embroidery: the stitch plan, machine profiles and stitches from paths"`, dependencies `vectorcraft-geom`, `vectorcraft-color`, `vectorcraft-doc`, `kurbo`; dev-dependency `proptest`. Register the workspace dependency and the layer table row.

- [ ] **Step 2: Write the failing tests** in `plan.rs` (`#[cfg(test)] mod tests`):

```rust
#[test] fn points_round_to_tenths_of_a_millimetre() {
    assert_eq!(Pt::from_mm(1.04, -2.06), Some(Pt { x: 10, y: -21 }));
    assert_eq!(Pt::from_pt(72.0, 36.0), Some(Pt { x: 254, y: 127 }));
    assert_eq!(Pt::from_mm(f64::NAN, 0.0), None);
    assert_eq!(Pt::from_mm(1.0e9, 0.0), None);
}
#[test] fn a_plan_counts_its_steps_and_refuses_too_many() {
    let block = Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt::default()); 3] };
    let plan = StitchPlan { name: "t".into(), blocks: vec![block.clone(), block] };
    assert_eq!(plan.steps(), 6);
    assert!(plan.check().is_ok());
    let big = StitchPlan { name: "t".into(), blocks: vec![Block { thread: Thread::default(), steps: vec![Step::Trim; MAX_STEPS + 1] }] };
    assert_eq!(big.check().unwrap_err(), "the design has more than 2000000 stitches");
}
```

- [ ] **Step 3: Run** `cargo test -p vectorcraft-stitch`. Expected: fails to compile (types missing).
- [ ] **Step 4: Implement** the types and methods above.
- [ ] **Step 5: Run** `cargo test -p vectorcraft-stitch` and `cargo xtask layers`. Expected: both pass.
- [ ] **Step 6: Commit** `E1.1: stitch plan model`.

---

### Task 2: Machine profiles, preparing a plan, statistics

**Files:**
- Create: `crates/stitch/src/profile.rs`, `crates/stitch/src/prepare.rs`, `crates/stitch/src/stats.rs`
- Modify: `crates/stitch/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct MachineProfile { pub id: &'static str, pub max_stitch: i32, pub max_jump: i32, pub min_stitch: i32, pub needles: Option<u8>, pub jumps_per_trim: u8, pub field_mm: Option<(f64, f64)> }
pub const GENERIC: MachineProfile;      // "generic": 121, 121, 3, None, 3, None
pub const COMMERCIAL15: MachineProfile; // "commercial15": 121, 121, 3, Some(15), 3, Some((500.0, 350.0))
pub fn profile(id: &str) -> Option<MachineProfile>;
pub const LOCK_LENGTH: i32 = 6; // units: 0.6 mm
#[derive(Clone, Copy, Debug)] pub struct PrepareOptions { pub locks: bool }   // Default: locks true
pub struct Prepared { pub plan: StitchPlan, pub warnings: Vec<String> }
pub fn prepare(plan: &StitchPlan, profile: &MachineProfile, o: &PrepareOptions) -> Result<Prepared, String>;
#[derive(Clone, Debug, PartialEq)] pub struct ThreadStats { pub rgb: [u8; 3], pub stitches: usize, pub length_mm: f64 }
#[derive(Clone, Debug, PartialEq)] pub struct Stats { pub stitches: usize, pub jumps: usize, pub trims: usize, pub color_changes: usize, pub width_mm: f64, pub height_mm: f64, pub threads: Vec<ThreadStats> }
pub fn stats(plan: &StitchPlan) -> Stats;
```

`prepare` applies the six passes in the spec's order ("Machine profile and preparing a plan"). Decisions the spec leaves to this plan:
- A run is a maximal sequence of consecutive `Stitch` steps inside one block; `Jump`, `Trim`, `Stop` and block boundaries end it.
- Long stitch from A to B (longer than `max_stitch`): replace `Stitch(B)` with `Jump(B)`, `Stitch(B)`. Pass 5 then splits the jump.
- Travel trim: a `Jump` whose larger axis distance from the current position needs `ceil(d / max_jump) >= jumps_per_trim` records gets a `Trim` inserted before it, unless the previous step is already `Trim` or nothing has been stitched yet in the plan.
- Lock at a run start with first points P0, P1: Q is the point `LOCK_LENGTH` units from P0 toward P1, or P1 if the segment is shorter; insert `Stitch(Q), Stitch(P0), Stitch(Q)` after `Stitch(P0)`. At a run end with last points Pm, Pn: Q' is `LOCK_LENGTH` from Pn toward Pm; append `Stitch(Q'), Stitch(Pn), Stitch(Q')` after `Stitch(Pn)`. Runs of fewer than two stitches get no locks.
- Jump split into k equal parts uses absolute interpolation, `A + (B - A) * i / k` rounded, so parts never exceed `max_jump`.
- Warnings, exact text: `"{n} thread colours, but the machine has {m} needles"`, `"the design is {w} × {h} mm, larger than the {fw} × {fh} mm field"` (one decimal place), `"colour {i} has no stitches"` (1-based).
- `stats`: `color_changes` is `blocks.len() - 1` (0 for an empty plan); `threads` merges blocks with the same `rgb` in first-seen order; `length_mm` sums stitch-to-stitch distances inside runs; width and height span every `Stitch` point.

- [ ] **Step 1: Write the failing tests** in `prepare.rs` and `stats.rs`. Helper `fn plan(steps: Vec<Step>) -> StitchPlan` builds a one-block plan; `S(x, y)` and `J(x, y)` abbreviate `Step::Stitch(Pt{..})` and `Step::Jump(Pt{..})`.

```rust
#[test] fn small_stitches_are_dropped_but_not_the_last() {
    let p = prepare(&plan(vec![S(0,0), S(1,0), S(30,0), S(31,0)]), &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
    assert_eq!(p.blocks[0].steps, vec![S(0,0), S(30,0), S(31,0)]);
}
#[test] fn a_long_stitch_becomes_jumps_then_a_stitch() {
    let p = prepare(&plan(vec![S(0,0), S(200,0)]), &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
    assert_eq!(p.blocks[0].steps, vec![S(0,0), J(100,0), J(200,0), S(200,0)]);
}
#[test] fn travel_of_three_jump_records_is_trimmed_first() {
    let p = prepare(&plan(vec![S(0,0), S(30,0), J(300,0), S(300,0), S(330,0)]), &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
    assert_eq!(p.blocks[0].steps, vec![S(0,0), S(30,0), Step::Trim, J(120,0), J(210,0), J(300,0), S(300,0), S(330,0)]);
}
#[test] fn runs_are_locked_at_both_ends() {
    let p = prepare(&plan(vec![S(0,0), S(30,0), S(60,0)]), &GENERIC, &PrepareOptions { locks: true }).unwrap().plan;
    assert_eq!(p.blocks[0].steps, vec![S(0,0), S(6,0), S(0,0), S(6,0), S(30,0), S(60,0), S(54,0), S(60,0), S(54,0)]);
}
#[test] fn needle_and_field_warnings() {
    let mut many = StitchPlan::default();
    for i in 0..16u8 { many.blocks.push(Block { thread: Thread { rgb: [i, 0, 0], ..Default::default() }, steps: vec![S(0,0), S(6000,0)] }); }
    let w = prepare(&many, &COMMERCIAL15, &PrepareOptions { locks: false }).unwrap().warnings;
    assert!(w.contains(&"16 thread colours, but the machine has 15 needles".to_string()));
    assert!(w.contains(&"the design is 600.0 × 0.0 mm, larger than the 500.0 × 350.0 mm field".to_string()));
}
#[test] fn stats_count_what_is_there() {
    let s = stats(&plan(vec![S(0,0), S(30,0), Step::Trim, J(100,40), S(100,40), S(100,70)]));
    assert_eq!((s.stitches, s.jumps, s.trims, s.color_changes), (4, 1, 1, 0));
    assert_eq!((s.width_mm, s.height_mm), (10.0, 7.0));
    assert_eq!(s.threads, vec![ThreadStats { rgb: [0,0,0], stitches: 4, length_mm: 6.0 }]);
}
```

Add one proptest in `prepare.rs`: for arbitrary plans (up to 3 blocks of up to 40 steps, coordinates in ±5000), after `prepare` with `GENERIC` every `Stitch` is at most 121 units on each axis from the previous position, every `Jump` likewise, and no three consecutive `Jump` steps occur without a `Trim` immediately before the run, except before the first stitch of the plan.

- [ ] **Step 2: Run** `cargo test -p vectorcraft-stitch`. Expected: compile failure.
- [ ] **Step 3: Implement** `profile.rs`, `prepare.rs` (one private function per pass), `stats.rs`.
- [ ] **Step 4: Run** `cargo test -p vectorcraft-stitch`. Expected: pass.
- [ ] **Step 5: Commit** `E1.2: machine profiles, plan preparation and statistics`.

---

### Task 3: The `embroidery` crate and DST

**Files:**
- Create: `crates/embroidery/Cargo.toml`, `crates/embroidery/src/lib.rs`, `crates/embroidery/src/dst.rs`, `crates/embroidery/src/dst/header.rs`, `crates/embroidery/src/dst/record.rs`, `crates/embroidery/src/palette.rs`
- Modify: `Cargo.toml` (workspace dependency `vectorcraft-embroidery`, next to `vectorcraft-cad`), `xtask/src/layers.rs` (append `("embroidery", Class::Layer(3))`)

**Interfaces:**
- Consumes: `vectorcraft_stitch::{StitchPlan, Block, Thread, Step, Pt, MAX_STEPS}`.
- Produces, in `lib.rs`:

```rust
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)] pub enum Start { #[default] Center, FirstStitch }
#[derive(Clone, Debug)] pub struct WriteOptions { pub start: Start, pub return_to_start: bool, pub jumps_per_trim: u8, pub trim_before_color_change: bool }
// Default: Center, true, 3, false
#[derive(Clone, Debug)] pub struct ReadOptions { pub jumps_per_trim: u8 }   // Default: 3
pub struct Output { pub bytes: Vec<u8>, pub warnings: Vec<String> }
pub struct Input { pub plan: StitchPlan, pub warnings: Vec<String> }
pub fn default_thread(index: usize) -> Thread;   // palette.rs
pub mod dst { pub fn is_dst(bytes: &[u8]) -> bool; pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String>; pub fn read(bytes: &[u8], o: &ReadOptions) -> Result<Input, String>; }
```

- `record.rs` (crate-private): `fn encode(dx: i32, dy: i32, flags: u8) -> Option<[u8; 3]>` (`None` outside ±121; `dy` is file orientation, y up; `flags` is `0x00` stitch, `0x80` jump) and `fn decode(b: [u8; 3]) -> (i32, i32)`, both from the bit table in the spec's DST section. Constants `COLOR_CHANGE: [u8; 3] = [0, 0, 0xC3]`, `END: [u8; 3] = [0, 0, 0xF3]`.
- `header.rs` (crate-private): `fn write(name: &str, records: usize, color_changes: usize, ext: Extents, end: (i32, i32)) -> [u8; 512]` and `fn read(h: &[u8]) -> Fields { name: String, threads: Vec<Thread> }`.
- `palette.rs`: twelve original colours, cycled by index: `[0,0,0] [200,30,40] [30,90,200] [20,140,70] [240,200,20] [240,120,20] [130,50,160] [120,70,30] [240,130,170] [40,170,190] [120,120,120] [250,250,250]`.
- Writer decisions:
  - The plan is written as given (the caller runs `prepare`). A move outside ±121 is the error `"a stitch is longer than the format allows: prepare the plan first"`.
  - Positions are relative to the origin: `Start::Center` is the midpoint of the extents of all `Stitch` points (integer division by 2); `Start::FirstStitch` is the first `Stitch` point. Leading travel from the origin to the first stitch, and trailing travel back with `return_to_start`, are jump records split into equal parts of at most 121.
  - A `Stitch` at the current position still writes a record (`00 00 03`).
  - `Trim` writes `jumps_per_trim` jump records alternating `(+2, 0)` and `(-2, 0)`, with the last one returning to the trim position: for 3 these are `(+2,0)`, `(-4,0)`, `(+2,0)`. A `jumps_per_trim` below 2 is the error `"jumpsPerTrim must be 2 or more"`.
  - `Stop` and each block boundary write `COLOR_CHANGE`. With `trim_before_color_change`, a trim is written first.
  - An empty plan (no `Stitch`) is the error `"nothing to stitch"`.
- Reader decisions: fewer than 512 bytes is the error `"not a DST file: the header is incomplete"`; more than `MAX_FILE_BYTES` is `"the file is larger than 64 MB"`; a trailing partial record is ignored with the warning `"the file ends in the middle of a stitch"`; a missing end record gives the warning `"the file has no end record"`. A run of `jumps_per_trim` or more jump records becomes `Trim` then one `Jump` to the run's final position (no `Jump` if the run returns to where it began). A `0x43` record is skipped and warns once: `"sequin records were skipped"`. Each colour change starts a new block; block `i` takes `TC:` thread `i` if the header has one, else `default_thread(i)`. Positions are converted to y down.

- [ ] **Step 1: Scaffold the crate** (`description = "VectorCraft embroidery machine files: DST and EXP readers and writers"`, dependency `vectorcraft-stitch`, dev-dependency `proptest`, `#![forbid(unsafe_code)]`).

- [ ] **Step 2: Write the failing tests.** In `record.rs`:

```rust
#[test] fn known_moves_have_known_bytes() {
    assert_eq!(encode(1, 0, 0), Some([0x01, 0x00, 0x03]));
    assert_eq!(encode(0, 1, 0), Some([0x80, 0x00, 0x03]));
    assert_eq!(encode(5, 0, 0), Some([0x06, 0x02, 0x03]));
    assert_eq!(encode(121, -121, 0), Some([0x55, 0x55, 0x17]));
    assert_eq!(encode(2, 0, 0x80), Some([0x02, 0x01, 0x83]));
    assert_eq!(encode(122, 0, 0), None);
}
#[test] fn every_move_round_trips() {
    for dx in -121..=121 { for dy in -121..=121 { assert_eq!(decode(encode(dx, dy, 0).unwrap()), (dx, dy)); } }
}
```

In `dst.rs`, with `fn tiny() -> StitchPlan` = name `"TEST"`, one block, `Stitch(0,0), Stitch(10,0), Stitch(10,10)`:

```rust
#[test] fn a_tiny_design_has_exact_bytes() {
    let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() };
    let b = write(&tiny(), &o).unwrap().bytes;
    let head = format!("LA:{:<16}\rST:{:>7}\rCO:{:>3}\r+X:{:>5}\r-X:{:>5}\r+Y:{:>5}\r-Y:{:>5}\rAX:+{:>5}\rAY:-{:>5}\rMX:+{:>5}\rMY:+{:>5}\rPD:******\r\x1a", "TEST", 4, 0, 10, 0, 0, 10, 10, 10, 0, 0);
    assert_eq!(&b[..head.len()], head.as_bytes());
    assert_eq!(head.len(), 125);
    assert!(b[125..512].iter().all(|c| *c == 0x20));
    assert_eq!(&b[512..], [0x00,0x00,0x03, 0x05,0x00,0x03, 0x50,0x00,0x03, 0x00,0x00,0xF3]);
}
#[test] fn the_name_is_sanitised_to_sixteen_ascii_characters() {
    let mut p = tiny(); p.name = "狗 portrait.final-version-2".into();
    let b = write(&p, &WriteOptions::default()).unwrap().bytes;
    assert_eq!(&b[..20], b"LA: portraitfinal-v\r");
    assert_eq!(&b[20..23], b"ST:");
}
#[test] fn trims_colour_changes_and_centre_start() {
    let mut p = tiny();
    p.blocks[0].steps.push(Step::Trim);
    p.blocks.push(Block { thread: Thread { rgb: [255,0,0], ..Default::default() }, steps: vec![Step::Jump(Pt{x:10,y:0}), Step::Stitch(Pt{x:10,y:0}), Step::Stitch(Pt{x:0,y:0})] });
    let b = write(&p, &WriteOptions::default()).unwrap().bytes;
    let recs: Vec<&[u8]> = b[512..].chunks(3).collect();
    assert_eq!(recs[0], [0x69, 0x41, 0x83]);                    // from the centre (5, 5) to the first stitch: a jump of (-5, +5) in file orientation
    assert!(recs.contains(&[0x00u8, 0x00, 0xC3].as_slice()));
    assert_eq!(recs.last().unwrap(), &[0x00u8, 0x00, 0xF3].as_slice());
    assert_eq!(&b[31..37], b"CO:  1");
    let back = read(&b, &ReadOptions::default()).unwrap().plan;
    assert_eq!(back.blocks.len(), 2);
    assert_eq!(back.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1);
}
#[test] fn odd_headers_and_truncated_files_are_survived() {
    assert_eq!(read(&[0u8; 100], &ReadOptions::default()).unwrap_err(), "not a DST file: the header is incomplete");
    let mut b = write(&tiny(), &WriteOptions::default()).unwrap().bytes;
    b[23..30].copy_from_slice(b"0000004");                        // zero-padded ST
    b.truncate(b.len() - 3);                                       // no end record
    b.push(0x01);                                                  // a partial record
    let i = read(&b, &ReadOptions::default()).unwrap();
    assert_eq!(i.plan.blocks[0].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count(), 3);
    assert!(i.warnings.contains(&"the file has no end record".to_string()));
    assert!(i.warnings.contains(&"the file ends in the middle of a stitch".to_string()));
}
```

Name rule for the sanitised test: keep ASCII letters, digits, space, `-` and `_`; drop every other character; then truncate to 16; then pad with spaces.

Add a proptest: for arbitrary plans prepared with `GENERIC` (`locks: false`), `read(write(plan))` has the same sequence of `Stitch` positions per block, relative to the first stitch.

- [ ] **Step 3: Run** `cargo test -p vectorcraft-embroidery`. Expected: compile failure.
- [ ] **Step 4: Implement** `record.rs`, `header.rs`, `palette.rs`, `dst.rs`.
- [ ] **Step 5: Run** `cargo test -p vectorcraft-embroidery`, `cargo xtask layers`, `cargo check -p vectorcraft-embroidery --target wasm32-unknown-unknown`. Expected: all pass.
- [ ] **Step 6: Commit** `E1.3: DST reader and writer`.

---

### Task 4: EXP

**Files:**
- Create: `crates/embroidery/src/exp.rs`
- Modify: `crates/embroidery/src/lib.rs`

**Interfaces:**
- Consumes: Task 3's `WriteOptions`, `ReadOptions`, `Output`, `Input`, `default_thread`.
- Produces: `pub mod exp { pub fn write(plan: &StitchPlan, o: &WriteOptions) -> Result<Output, String>; pub fn read(bytes: &[u8], o: &ReadOptions) -> Result<Input, String>; }`

Decisions: byte meanings are the table in the spec's EXP section. Moves are limited to ±127 per axis, with the same "prepare the plan first" error as DST. `Start` and `return_to_start` behave as in DST, with travel as `80 04 dx dy` records. `jumps_per_trim` is ignored on write (the trim has its own code) and on read. The reader turns a jump run into one `Jump` to its final position. An unknown control byte ends reading with the warning `"an unknown code ended the file early"`.

- [ ] **Step 1: Write the failing tests** (`tiny()` as in Task 3):

```rust
#[test] fn a_tiny_design_has_exact_bytes() {
    let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() };
    assert_eq!(write(&tiny(), &o).unwrap().bytes, [0x00,0x00, 0x0A,0x00, 0x00,0xF6]);
}
#[test] fn jumps_trims_and_colour_changes_have_their_codes() {
    let mut p = tiny();
    p.blocks[0].steps.extend([Step::Trim, Step::Jump(Pt{x:30,y:10}), Step::Stitch(Pt{x:30,y:10})]);
    p.blocks.push(Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt{x:30,y:20})] });
    let o = WriteOptions { start: Start::FirstStitch, return_to_start: false, ..Default::default() };
    assert_eq!(write(&p, &o).unwrap().bytes,
        [0x00,0x00, 0x0A,0x00, 0x00,0xF6, 0x80,0x80,0x07,0x00, 0x80,0x04,0x14,0x00, 0x00,0x00, 0x80,0x01,0x00,0x00, 0x00,0xF6]);
}
#[test] fn the_reader_is_lenient() {
    let bytes = [0x00,0x00, 0x80,0x02,0x05,0x00, 0x80,0x06,0x05,0x00, 0x00,0x00, 0x80,0x10,0x00,0x00, 0x1A];
    let i = read(&bytes, &ReadOptions::default()).unwrap();
    assert_eq!(i.plan.blocks[0].steps, vec![Step::Stitch(Pt{x:0,y:0}), Step::Stitch(Pt{x:5,y:0}), Step::Jump(Pt{x:10,y:0}), Step::Stitch(Pt{x:10,y:0})]);
    assert!(i.warnings.is_empty());
    let cut = read(&[0x00,0x00, 0x80,0x7E,0x00,0x00, 0x05,0x00], &ReadOptions::default()).unwrap();
    assert_eq!(cut.warnings, vec!["an unknown code ended the file early".to_string()]);
}
```

Add the same round-trip proptest as DST, with the profile's maxima raised to 127.

- [ ] **Step 2: Run** `cargo test -p vectorcraft-embroidery exp`. Expected: compile failure.
- [ ] **Step 3: Implement** `exp.rs`.
- [ ] **Step 4: Run** `cargo test -p vectorcraft-embroidery`. Expected: pass.
- [ ] **Step 5: Commit** `E1.4: EXP reader and writer`.

---

### Task 5: Documents to plans and back

**Files:**
- Create: `crates/stitch/src/from_doc.rs`, `crates/stitch/src/to_doc.rs`, `crates/stitch/src/run.rs`
- Modify: `crates/stitch/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 types; `vectorcraft_doc::{Document, Node, NodeKind, NodeId, Appearance, Unit}`, `vectorcraft_geom::{PathData, SubPath, flatten}`, `vectorcraft_color::Paint`. Open `crates/doc/src/node/kind.rs`, `crates/doc/src/appearance.rs` (`stroke_paint`, `stroke`), `crates/geom/src/path.rs` (`SubPath::polyline`, `to_bezpath`) before using them; confirm how a `Color` converts to 8-bit RGB in `crates/color/src/lib.rs` and use that function.
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)] pub enum Mode { #[default] Run, Manual }
#[derive(Clone, Debug)] pub struct FromDocOptions { pub mode: Mode, pub stitch_length: i32 /* units, default 25 */, pub min_stitch: i32 /* default 3 */, pub only: Option<Vec<NodeId>> }
pub struct Planned { pub plan: StitchPlan, pub warnings: Vec<String> }
pub fn from_document(doc: &Document, o: &FromDocOptions) -> Result<Planned, String>;
pub fn to_document(plan: &StitchPlan) -> Document;
pub fn run_stitches(points_mm: &[(f64, f64)], corners: &[bool], closed: bool, stitch_length: i32) -> Vec<Pt>; // run.rs
pub const THREAD_WIDTH_MM: f64 = 0.4;
```

Decisions:
- `run_stitches` takes a flattened polyline in mm with a corner flag per vertex. It emits the first vertex, then walks each stretch between consecutive corner vertices (the ends count as corners), dividing its length into `ceil(length / stitch_length)` equal steps placed by arc length, so every corner and the last vertex are hit exactly. `closed` appends the first vertex as the final target. Zero-length input gives an empty result.
- Flattening tolerance: 0.05 mm, converted to document points. Vertices produced inside a curve are not corners; every anchor of the path is a corner only if its `kind` is a corner kind. Check the `Anchor` kind enum for the exact variant names.
- `Mode::Manual`: one point per anchor, in order; a closed subpath appends its first anchor.
- The walk: `doc.layers` bottom to top (confirm the order `layers` is stored in against how the renderer paints, and say which in a comment), recursing `Layer` and `Group` children; skip nodes with `visible == false`; with `only`, stitch a node only if it or an ancestor is listed. `Path` and `Compound` with a stroke whose paint is not `Paint::None` are stitched; each subpath is one run, with `Trim` then `Jump` between runs unless the next start is within `min_stitch` of the current position.
- Warnings, exact text: `"{n} filled objects without a stroke were not stitched"`, `"{n} objects of other kinds (type, images, symbols) were not stitched"`, `"a stroke with a gradient or pattern was stitched in black"` (once).
- No stitches at all: `Err("nothing to stitch: the document has no stroked paths")`.
- The plan name is empty; the engine sets it.
- `to_document`: units `Unit::Millimeters`; one layer named `"Stitches"`; per block a group named `"Colour {i}"` (1-based) of open paths, one per run, corner anchors at the needle points, appearance `Appearance::basic(Paint::None, solid(thread.rgb), THREAD_WIDTH_MM in points)`; one artboard equal to the stitch extents grown by 10 mm on each side. Find how a document and an artboard are constructed in `crates/doc/src/lib.rs` and how the DXF importer does it in `crates/cad/src/import.rs`, and follow that.

- [ ] **Step 1: Write the failing tests.** In `run.rs`:

```rust
#[test] fn a_straight_line_is_stepped_evenly_and_ends_exactly() {
    let p = run_stitches(&[(0.0, 0.0), (10.0, 0.0)], &[true, true], false, 25);
    assert_eq!(p, vec![Pt{x:0,y:0}, Pt{x:25,y:0}, Pt{x:50,y:0}, Pt{x:75,y:0}, Pt{x:100,y:0}]);
    let q = run_stitches(&[(0.0, 0.0), (9.0, 0.0)], &[true, true], false, 25);
    assert_eq!(q.last(), Some(&Pt{x:90,y:0}));
    assert_eq!(q.len(), 5);
}
#[test] fn corners_always_get_a_needle_point() {
    let p = run_stitches(&[(0.0,0.0), (3.0,0.0), (3.0,3.0)], &[true, true, true], false, 25);
    assert!(p.contains(&Pt{x:30,y:0}));
    assert_eq!(p.last(), Some(&Pt{x:30,y:30}));
}
#[test] fn rounding_does_not_drift() {
    let p = run_stitches(&[(0.0,0.0), (1000.03,0.0)], &[true, true], false, 25);
    assert_eq!(p.last(), Some(&Pt{x:10000,y:0}));
}
```

In `from_doc.rs`, building documents with the doc crate's own constructors (`Node::path`, `Node::group`, `Node::layer`, `Appearance::basic`); a helper `fn square_mm(side: f64) -> PathData` makes a closed square with corner anchors, in points:

```rust
#[test] fn closed_path_returns_to_its_start() {
    let plan = from_document(&doc_with(vec![stroked(square_mm(10.0), RED)]), &FromDocOptions::default()).unwrap().plan;
    let pts: Vec<Pt> = plan.points().collect();
    assert_eq!(pts.first(), pts.last());
    assert_eq!(pts.len(), 17);                         // 4 sides × 4 steps of 2.5 mm, plus the start
}
#[test] fn colours_make_blocks_and_objects_are_trimmed_apart() {
    let d = doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED), stroked(line_mm(0.0, 20.0, 10.0, 20.0), RED), stroked(line_mm(0.0, 40.0, 10.0, 40.0), BLUE)]);
    let plan = from_document(&d, &FromDocOptions::default()).unwrap().plan;
    assert_eq!(plan.blocks.len(), 2);
    assert_eq!(plan.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1);
    assert_eq!(plan.blocks[0].thread.rgb, [255, 0, 0]);
}
#[test] fn fills_hidden_art_and_other_kinds_are_skipped_with_warnings() {
    // a filled square with no stroke, a hidden stroked line, a visible stroked line
    let out = from_document(&mixed_doc(), &FromDocOptions::default()).unwrap();
    assert_eq!(out.plan.blocks.len(), 1);
    assert_eq!(out.warnings, vec!["1 filled objects without a stroke were not stitched".to_string()]);
}
#[test] fn degenerate_paths_add_nothing() {
    let d = doc_with(vec![stroked(single_anchor(), RED), stroked(line_mm(5.0, 5.0, 5.0, 5.0), RED)]);
    assert_eq!(from_document(&d, &FromDocOptions::default()).unwrap_err(), "nothing to stitch: the document has no stroked paths");
}
#[test] fn manual_mode_uses_the_anchors() {
    let o = FromDocOptions { mode: Mode::Manual, ..Default::default() };
    let plan = from_document(&doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]), &o).unwrap().plan;
    assert_eq!(plan.blocks[0].steps, vec![Step::Stitch(Pt{x:0,y:0}), Step::Stitch(Pt{x:100,y:0})]);
}
```

In `to_doc.rs`:

```rust
#[test] fn a_plan_round_trips_through_a_document() {
    let plan = /* two blocks; the first has two runs separated by Trim and Jump */;
    let doc = to_document(&plan);
    let back = from_document(&doc, &FromDocOptions { mode: Mode::Manual, ..Default::default() }).unwrap().plan;
    let stitches = |p: &StitchPlan| p.blocks.iter().map(|b| b.steps.iter().filter_map(|s| if let Step::Stitch(q) = s { Some(*q) } else { None }).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(stitches(&back), stitches(&plan));
    assert_eq!(back.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>(), plan.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>());
}
```

Write the elided plan literal in full in the test.

- [ ] **Step 2: Run** `cargo test -p vectorcraft-stitch`. Expected: compile failure.
- [ ] **Step 3: Implement** `run.rs`, then `from_doc.rs`, then `to_doc.rs`.
- [ ] **Step 4: Run** `cargo test -p vectorcraft-stitch` and `cargo check -p vectorcraft-stitch --target wasm32-unknown-unknown`. Expected: pass.
- [ ] **Step 5: Commit** `E1.5: stitches from documents and documents from stitches`.

---

### Task 6: Engine formats, export, open and `stitch.info`

**Files:**
- Create: `crates/engine/src/cmd/fileio/stitchfile.rs`, `crates/engine/src/cmd/fileio/tests_stitchfile.rs`, `crates/engine/src/cmd/stitch.rs`
- Modify: `crates/engine/Cargo.toml` (dependencies `vectorcraft-stitch`, `vectorcraft-embroidery`), `crates/engine/src/cmd/fileio.rs` (declare the two modules, extend `specs()` if `stitchfile` has any), `crates/engine/src/cmd/fileio/formats.rs` (two `Format` rows after `dxf`; add `"dst"` and `"exp"` after `"dxf"` in both extension lists near lines 551 and 640), `crates/engine/src/cmd/fileio/encode.rs` (match arm next to `"dxf"` in `encode_files`), `crates/engine/src/cmd/fileio/load.rs` (sniff next to `is_dxf` near line 93; import arm next to `"dxf"` near line 465; a `stitch` field in the open options struct next to `dxf`), `crates/engine/src/cmd.rs` (`mod stitch;` and `v.extend(stitch::specs());`)

**Interfaces:**
- Consumes: `vectorcraft_stitch::{from_document, to_document, prepare, stats, profile, FromDocOptions, Mode, PrepareOptions, Stats}`, `vectorcraft_embroidery::{dst, exp, WriteOptions, ReadOptions, Start}`. Engine patterns to copy: `crates/engine/src/cmd/fileio/dxf.rs` (`OPTIONS`, a serde `Params` struct, `encode`) and `dxfimport.rs` (`options`, `import`); `crates/engine/src/cmd/halftone.rs` for a small command module.
- Produces:
  - `stitchfile::OPTIONS: &[FormatOption]` with these names, types and defaults: `machine` string `"commercial15"`; `mode` string `"run"`; `stitchLength` number `2.5`; `minStitch` number `0.3`; `maxStitch` number `12.1` (`12.7` for EXP, stated in the description); `locks` boolean `true`; `jumpsPerTrim` integer `3`; `trimBeforeColorChange` boolean `false`; `start` string `"center"`; `returnToStart` boolean `true`; `name` string `null`; `selectedOnly` boolean `false`. Lengths are in mm.
  - `stitchfile::encode(doc: &Document, p: &Value, format: &str, selection: Option<Vec<NodeId>>) -> Result<Encoded>`; find how `dxf::encode` receives `selectedOnly` and pass the selection the same way.
  - `stitchfile::plan(doc, p, format, selection) -> Result<(StitchPlan, Vec<String>)>`: `from_document`, set the name, `prepare`; shared by export and `stitch.info`.
  - `stitchfile::import(bytes: &[u8], format: &str, p: Option<&Value>) -> Result<(Document, Vec<String>)>`.
  - Format rows: `Format { id: "dst", label: "DST stitch file", extensions: &["dst"], mime: "application/octet-stream", read: true, write: true, raster: false, options: stitchfile::OPTIONS }` and the same for `exp` with label `"EXP stitch file"`.
  - Command `stitch.info`, a `query` with label `"Stitch Info"`, menu `[]`, enabled `has_doc`, params doc listing the export options plus `format?: dst (default) | exp`, returning `{stitches, jumps, trims, colorChanges, widthMm, heightMm, threads: [{rgb: "#rrggbb", stitches, lengthMm}], warnings}`.

Decisions:
- Option ranges, each refused with `bad(C, …)` naming the range: `stitchLength` 0.5–12; `minStitch` 0–2; `maxStitch` 1–12.1 (12.7 for EXP); `jumpsPerTrim` 2–8; `machine` one of `generic`, `commercial15`; `mode` one of `run`, `manual`; `start` one of `center`, `firstStitch`. Non-finite numbers are refused. The serde struct uses `deny_unknown_fields` only if `dxf.rs` refuses unknown keys that way; otherwise use the mechanism `dxf.rs` uses.
- The profile is `profile(machine)` with `max_stitch`, `max_jump`, `min_stitch` and `jumps_per_trim` overridden by the options, and both maxima capped at 127 for EXP and 121 for DST.
- The plan name is `name` if given, else the document's title without its extension (find how `dxf.rs` or `export.rs` reads the title), else `"design"`.
- Sniffing: `dst::is_dst(bytes)` selects `dst`; `exp` is chosen only by the `.exp` extension, through the existing by-name path.
- `import` passes `{jumpsPerTrim}` from the `stitch` open option, range 2–8.

- [ ] **Step 1: Write the failing tests** in `tests_stitchfile.rs`, using the session helpers the neighbouring `tests_dxf.rs` uses (open that file first and copy its setup):

```rust
#[test] fn a_stroked_rectangle_exports_and_reopens() {
    // new document; shape.rectangle {x:0,y:0,width:100,height:50} with a red stroke; export dst with locks false
    // assert: the reply has no error; bytes start with b"LA:"; reopening via document.open {name:"a.dst", dataBase64} gives a document
    // whose stitch.info reports the same `stitches` as stitch.info on the original with the same options
}
#[test] fn exp_exports_by_format_and_opens_by_extension() { /* same, format exp, name "a.exp" */ }
#[test] fn stitch_info_reports_counts_and_size() {
    // one horizontal stroked line 100 mm long, options {locks:false, stitchLength:2.5}
    // assert: stitches == 41, jumps == 0, trims == 0, colorChanges == 0, widthMm == 100.0, heightMm == 0.0, threads.len() == 1
}
#[test] fn bad_stitch_options_are_refused() {
    for p in [json!({"stitchLength": 0}), json!({"stitchLength": -1}), json!({"stitchLength": "2"}), json!({"stitchLength": 1e400}), json!({"jumpsPerTrim": 1}), json!({"machine": "x"}), json!({"mode": "x"}), json!({"nope": 1})] {
        // document.export {format:"dst", ..p} is an error whose message names the option
    }
}
#[test] fn an_empty_document_is_an_error_not_an_empty_file() {
    // message contains "nothing to stitch"
}
#[test] fn the_formats_are_listed() {
    // document.formats lists dst and exp with read and write true
}
```

Write each test body in full against the real helper names.

- [ ] **Step 2: Run** `cargo test -p vectorcraft-engine stitchfile`. Expected: compile failure.
- [ ] **Step 3: Implement** `stitchfile.rs`, the format rows, the `encode_files` arm, the load sniff and import arm, and `cmd/stitch.rs` with `stitch.info`.
- [ ] **Step 4: Run** `cargo test -p vectorcraft-engine stitchfile` and `cargo test -p vectorcraft-engine --test command_sweep`. Expected: pass.
- [ ] **Step 5: Check the CLI end to end.** Run:

```bash
cargo run -q -p vectorcraft-cli -- run --cmd shape.rectangle --params '{"x":10,"y":10,"width":200,"height":100}' --export "$SCRATCH/r.dst"
cargo run -q -p vectorcraft-cli -- info "$SCRATCH/r.dst"
```

Expected: the first writes a file larger than 512 bytes; the second describes a document with one layer. If the default rectangle has no stroke, set one in the params or with a second `--cmd`; check `vectorcraft-cli run --help` for how several commands are passed.

- [ ] **Step 6: Commit** `E1.6: DST and EXP in export and open, stitch.info`.

---

### Task 7: The calibration design

**Files:**
- Create: `crates/stitch/src/calibration.rs`
- Modify: `crates/stitch/src/lib.rs`, `crates/engine/src/cmd/stitch.rs`, `crates/engine/src/cmd/fileio/tests_stitchfile.rs`

**Interfaces:**
- Consumes: `PathData`, `SubPath`, `Anchor` from `vectorcraft_geom`.
- Produces: `pub struct CalibrationPart { pub name: &'static str, pub path: PathData /* points, y down */, pub rgb: [u8; 3] }` and `pub fn calibration() -> Vec<CalibrationPart>`; command `stitch.calibration`, label `"Stitch Calibration Design"`, menu `[]`, enabled `has_doc`, params `{x?: 0, y?: 0 (top-left corner, document units)}`, returning `{id}` of the new group.

The design, in mm from its top-left corner, in stitching order:
1. `"square"`, black `[0,0,0]`: closed square from (0,0) to (100,100).
2. `"ticks"`, black: eleven open lines along the top edge at x = 0, 10, …, 100, from y = 0 to y = 3 (y = 6 at x = 0, 50 and 100).
3. `"F"`, red `[200,30,40]`: one open polyline (5,35) → (5,10) → (20,10) → (5,10) → (5,22) → (15,22), so the letter reads upright with its arms to the right.
4. `"near pair"`, blue `[30,90,200]`: two closed 10 mm squares with top-left corners (30,40) and (45,40): 5 mm apart.
5. `"far pair"`, blue: two closed 10 mm squares with top-left corners (30,70) and (70,70): 30 mm apart.

Every anchor is a corner. The first stitch is the square's corner at (0,0), which is off-centre.

The command adds one group named `"Stitch Calibration"` holding one path per part, stroked `THREAD_WIDTH_MM` in the part's colour with no fill, in one undo step (`s.edit`), as `crates/engine/src/cmd/halftone.rs` inserts its group.

- [ ] **Step 1: Write the failing tests.** In `calibration.rs`:

```rust
#[test] fn the_design_is_100_mm_square_in_three_colours() {
    let parts = calibration();
    let mut colours: Vec<[u8; 3]> = parts.iter().map(|p| p.rgb).collect(); colours.dedup();
    assert_eq!(colours.len(), 3);
    let b = parts.iter().fold(None::<vectorcraft_geom::Rect>, |a, p| /* union of p.path.bounds() */);
    // width and height are both 100 mm in points: 100.0 * 72.0 / 25.4, within 1e-6
}
```

In `tests_stitchfile.rs`:

```rust
#[test] fn the_calibration_design_stitches_as_documented() {
    // new document; stitch.calibration {}; stitch.info {locks:false}
    // assert: colorChanges == 2, widthMm == 100.0, heightMm == 100.0, trims >= 12 (ticks and squares are separate runs), warnings is empty
    // undo removes the group in one step
}
```

Write both bodies in full.

- [ ] **Step 2: Run** `cargo test -p vectorcraft-stitch calibration` and `cargo test -p vectorcraft-engine calibration`. Expected: compile failure.
- [ ] **Step 3: Implement** `calibration.rs` and the command.
- [ ] **Step 4: Run** the same tests and `cargo test -p vectorcraft-engine --test command_sweep`. Expected: pass.
- [ ] **Step 5: Commit** `E1.7: calibration design for the first stitch-out`.

---

### Task 8: Hostile input, docs and the full gate

**Files:**
- Modify: `crates/engine/tests/import_fuzz.rs`, `docs/embroidery-parity.md`, `docs/architecture.md`, `docs/development.md`

**Interfaces:**
- Consumes: `vectorcraft_embroidery::{dst, exp}`, `document.open`, `stitch.info`.

- [ ] **Step 1: Add fuzz cases** to `import_fuzz.rs`, modelled on `dxf_garbage_never_panics` and `survive_dxf` near line 2730. Three properties, each opening the bytes through `document.open` (names `x.dst` and `x.exp`) and calling the crate readers directly, all through `no_panic`:
  - `dst_garbage_never_panics`: arbitrary bytes 0–2000 long, with and without a valid 512-byte header in front.
  - `exp_garbage_never_panics`: arbitrary bytes 0–2000 long.
  - `mutated_stitch_files_never_panic`: a valid file written from the calibration design with one to eight bytes replaced at arbitrary offsets.
- [ ] **Step 2: Run** `cargo test -p vectorcraft-engine --test import_fuzz stitch dst exp` (run the three by name). Expected: pass. A failure here is a reader bug: fix it in the format crate and add the failing input as a unit test there.
- [ ] **Step 3: Update the docs.**
  - `docs/embroidery-parity.md`: status line and revision history; Summary's measured figure; rows "Stitch plan model", "Machine profile and limits", "Running stitches" (partial: run and manual only), "Small-stitch filter", "Lock stitches" (partial: fixed pattern), "DST", "EXP", "Reading stitch files" marked done or partial with what is missing; gaps E1 (done) and E2 (DST and EXP done; hours reduced to the remaining formats).
  - `docs/architecture.md`: the two crates in the layer list with one line each.
  - `docs/development.md`: a short section "Checking stitch files against another reader" describing the manual cross-check: write a DST and an EXP with the CLI, read them with the MIT-licensed `pyembroidery` package from PyPI used as a black box, compare stitch counts and positions, and the reverse. State that it is not part of CI.
- [ ] **Step 4: Run the full gate:** `cargo xtask ci`. Expected: every stage passes (fmt, clippy, tests, layers, wasm, brands, assets). Fix what it reports and run it again.
- [ ] **Step 5: Do the manual cross-check** from Step 3 if Python and network access to PyPI are available; record in the commit message whether it was run and what it showed. If it could not be run, say so.
- [ ] **Step 6: Commit** `E1.8: stitch file fuzzing and docs`.

---

## After the plan: stitch-out

Not a code task. Export the calibration design to DST with the defaults, load it on the machine, and record the checklist in the spec's "Stitch-out acceptance". Findings that contradict a default (`ST:` count, jumps per trim, start position, y direction) become follow-up fixes, each with a test.
