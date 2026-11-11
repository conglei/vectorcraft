# Embroidery stitch core and DST/EXP files: design

> **Status:** draft for owner review · **Date:** 2026-11-11 · **Covers:** gap E1 and the DST/EXP part of E2 in [embroidery-parity.md](../../embroidery-parity.md)

## Goal

Draw stroked paths in VectorCraft, export a DST or EXP file, and stitch it on a commercial
multi-needle machine. Open a DST or EXP file and see its stitches. Everything later (fills, satin,
lettering, photos) builds on the stitch plan defined here.

The first test machine is a BAi Mirror, 15 needles. What its documentation says, and what it does
not say, is in [Machine facts](#machine-facts).

## What the owner decided

- Embroidery ships as a sibling app that reuses VectorCraft's vector editing. This spec adds no app:
  it adds library crates and engine commands that both apps will use. See
  [Open decisions](#open-decisions) for where the sibling app lives.
- Commercial multi-needle machines first, so DST and EXP before the home formats.
- Real stitch-outs are part of acceptance.

## Out of scope

Fill, satin and decorative stitches (E3, E13). Stitch properties stored per object and a parameters
panel (E4). Preview and simulator (E5). Thread charts and needle assignment (E6). Hoops, worksheets
and estimates (E7). Routing (E8). The other formats (rest of E2). The sibling app's shell and UI.
Any network or image-model code.

## Architecture

Two new crates and one engine module. Both crates must build for `wasm32`.

| Unit | Layer | Depends on | Does |
|---|---|---|---|
| `crates/stitch` (`vectorcraft-stitch`) | L2 | `geom`, `color`, `doc`, `kurbo` | The stitch plan, machine profiles, preparing a plan for a machine, statistics, running stitch along a path, a plan from a document, a document from a plan, the calibration design |
| `crates/embroidery` (`vectorcraft-embroidery`) | L3 | `stitch` | DST and EXP readers and writers. No document types |
| `crates/engine/src/cmd/fileio/stitchfile.rs` and `cmd/stitch.rs` | L5 | both | Options parsing, format table rows, export and open hooks, the `stitch.*` commands |

Both crates are registered in `xtask/src/layers.rs` (append-only table). `stitch` needs no
intra-layer entry. New crate roots carry `[lints] workspace = true` and the crate-level `deny`
attributes copied from an existing crate.

The split keeps each part testable alone: the format crate turns a plan into bytes and back and
knows nothing about documents; the stitch crate knows nothing about file layouts.

## The stitch plan

```text
StitchPlan { name: String, blocks: Vec<Block> }
Block      { thread: Thread, steps: Vec<Step> }
Thread     { rgb: [u8; 3], name: Option<String>, code: Option<String> }
Step       = Stitch(Pt) | Jump(Pt) | Trim | Stop
Pt         { x: i32, y: i32 }
```

- **Units.** `Pt` is an absolute position in units of 0.1 mm, the unit of every target format.
  Generators round absolute positions, so rounding never accumulates along a path.
- **Axes.** x to the right, y down, as in documents. Writers flip y, because both file formats have
  y up.
- **Blocks.** One block is one run of one thread. A colour change is the boundary between blocks.
  `Stop` is a pause inside a block (appliqué later); DST writes it as the same record as a colour
  change.
- **Stitch** moves and puts the needle down. **Jump** moves with the needle up. **Trim** cuts the
  thread at the current position.
- **Limits.** A plan holds at most 2,000,000 steps. Builders and readers return an error past that.

## Machine profile and preparing a plan

```text
MachineProfile { id, max_stitch, max_jump, min_stitch, needles, jumps_per_trim, field_mm: (w, h) }
```

Built-in profiles use neutral ids, because user-visible strings may not name vendors:

| id | max stitch | max jump | min stitch | needles | jumps per trim | field |
|---|---|---|---|---|---|---|
| `generic` | 12.1 mm | 12.1 mm | 0.3 mm | — | 3 | none |
| `commercial15` | 12.1 mm | 12.1 mm | 0.3 mm | 15 | 3 | 500 × 350 mm |

`commercial15` holds the values documented for the test machine; every field can be overridden by
an export option. EXP export raises the two maxima to 12.7 mm.

`prepare(plan, profile, options) -> Prepared { plan, warnings }` makes a plan safe for a machine,
in this order:

1. **Small stitches.** A stitch closer than `min_stitch` to the last kept needle point is dropped,
   except the last stitch of a run.
2. **Lock stitches** (option `locks`, on by default). At the start of each run (plan start, after a
   trim, after a colour change) and at its end, three stitches of 0.6 mm back and forth along the
   run's first or last segment. The length and count are estimates to tune at the stitch-out.
3. **Long stitches.** A stitch longer than `max_stitch` becomes jumps to within range and then a
   stitch, so the needle never comes down at a point the design did not ask for.
4. **Long jumps.** A jump longer than `max_jump` is split into equal parts.
5. **Travel that would read as a trim.** A machine set to N jumps per trim cuts the thread when it
   sees N jump records in a row. A travel that needs N or more jump records therefore gets an
   explicit `Trim` before it, and a lock stitch after it, so the file says what the machine will do.
6. **Checks, as warnings.** More distinct threads than `needles`; extents larger than `field_mm`;
   no stitches in a block.

`stats(plan)` returns stitch, jump, trim and colour-change counts, width and height in mm, and per
thread: colour, stitch count and stitched length in mm. Thread consumption and run time are E7.

## Stitches from a document

Until stitch objects exist (E4), stitches come from a document by one fixed rule.

- The walk covers visible art bottom to top, through layers and groups. With `selectedOnly`, only
  the selection.
- A path or compound path **with a stroke** is stitched along its outline. The thread colour is the
  stroke's solid colour. A gradient or pattern stroke is stitched in black, with a warning.
- **Mode `run`** (default): the path is flattened (tolerance 0.05 mm) and stepped at `stitchLength`
  (default 2.5 mm, allowed 0.5–12 mm). Corner anchors always get a needle point. The last step of a
  segment is shortened so the path's end is hit exactly.
- **Mode `manual`**: every anchor is one needle point and curves are ignored. This is what an
  opened stitch file re-exports through, unchanged.
- Fills are not stitched. A filled object with no stroke is skipped, with one warning that counts
  them. Type, images, symbols and other object kinds are skipped the same way.
- Consecutive objects with the same colour share a block. Between two objects the plan trims and
  jumps, unless the end of one is within `min_stitch` of the start of the next.
- A colour different from the previous object's starts a new block. Objects are never reordered.
- A document with nothing to stitch is an error, not an empty file.

Document units are points; 1 pt is 25.4/72 mm.

## A document from a plan

Opening a stitch file gives a new document in millimetres.

- One layer per file, one group per block named `Colour 1`, `Colour 2`, and so on.
- Each run of stitches between trims or jumps is an open path whose anchors are the needle points,
  stroked 0.4 mm in the thread colour.
- Jumps are not drawn.
- The artboard is the design's extents plus a 10 mm margin.

Exporting that document with mode `manual` reproduces the needle points. Trims and lock stitches
may differ; export with `locks: false` to keep the file's own.

## DST

Layout facts are from two permissively licensed implementations (pyembroidery, MIT; libembroidery,
zlib), read as format references and restated as facts. No GPL source was used. Test bytes are
written by hand from the tables in this section.

**Header.** 512 bytes of ASCII. Each field is a two-letter label, a colon, a fixed-width value and
0x0D. One 0x1A follows the last field (byte 125) and 0x20 fills the rest.

| Field | Width | Written as |
|---|---|---|
| `LA:` | 16 | The design name: ASCII letters, digits, space, `-` and `_` only, truncated, space-padded on the right |
| `ST:` | 7 | Number of 3-byte records in the file, including the end record; right-aligned, space-padded |
| `CO:` | 3 | Number of colour-change records |
| `+X:` `-X:` `+Y:` `-Y:` | 5 each | Extents from the start point in 0.1 mm, file orientation (y up) |
| `AX:` `AY:` | 6 each | End point relative to the start: a sign, then a 5-wide number |
| `MX:` `MY:` | 6 each | `+    0` |
| `PD:` | 6 | `******` |

No extended fields (`AU:`, `CP:`, `TC:`) are written: reports say some machines reject them. The
reader accepts them, and takes thread colours from `TC:` lines when present.

**Records.** Three bytes, a move of up to ±121 units per axis as a sum of ±1, ±3, ±9, ±27, ±81:

| Bit | Byte 0 | Byte 1 | Byte 2 |
|---|---|---|---|
| 0x01 | x +1 | x +3 | always set |
| 0x02 | x −1 | x −3 | always set |
| 0x04 | x +9 | x +27 | x +81 |
| 0x08 | x −9 | x −27 | x −81 |
| 0x10 | y −9 | y −27 | y −81 |
| 0x20 | y +9 | y +27 | y +81 |
| 0x40 | y −1 | y −3 | colour change or stop |
| 0x80 | y +1 | y +3 | jump |

- A stitch has neither flag. A jump has byte 2 with 0x83. A colour change or stop is `00 00 C3`,
  always with no move. The end is `00 00 F3`, and nothing follows it.
- A trim is `jumps_per_trim` jump records whose moves are each at most 0.4 mm, never zero, and sum
  to zero.
- No trim is written before a colour change: the machine cuts there anyway. Option
  `trimBeforeColorChange` turns it on.

**Start and end.** Option `start`: `center` (default) puts the file's origin at the centre of the
stitch extents, so the file opens with jumps from the centre to the first stitch; `firstStitch`
puts the origin at the first needle point. Option `returnToStart` (default on) ends with jumps back
to the origin, so `AX` and `AY` are zero.

**Reader.** Requires 512 bytes of header and reads the name from `LA:`; counts and extents in the
header are not trusted. Decoding stops at the end record or at a short read. A run of
`jumpsPerTrim` (default 3) or more jumps becomes a trim followed by one jump. A colour change with
a move applies the move. Sequin records are skipped, with a warning. Threads with no colour in the
file get colours from a small original built-in list, in order. Input over 64 MB or 2,000,000
records is an error.

`is_dst(bytes)`: the first three bytes are `LA:`. Otherwise the `.dst` extension decides.

## EXP

No header. Positions as in DST: 0.1 mm, y up, moves relative.

| Bytes | Meaning | Written |
|---|---|---|
| `dx dy` | stitch; two signed bytes, each −127 to +127 | yes |
| `80 04 dx dy` | jump | yes |
| `80 01 00 00` | colour change or stop | yes |
| `80 80 07 00` | trim | yes |

Nothing marks the end of the file and none is written. No colours are stored; companion colour
files are a later task. A jump runs all the way to the target and the stitch after it is `00 00`.

**Reader**, lenient because the two reference implementations disagree: `80 02 dx dy` is a stitch;
`80 04` and `80 06` are jumps; any other control byte with bit 0 set is a stop; `80 10` ends the
file; a single trailing `1A` is ignored; an unknown control byte ends reading with a warning. The
same size limits as DST apply.

EXP has no signature, so only the `.exp` extension selects it.

## Commands and options

Everything is reachable from the CLI, MCP and both apps, because it goes through the engine.

- **Format table.** Two rows, `dst` and `exp`, both read and write, labelled by extension
  (`DST stitch file`, `EXP stitch file`). `vectorcraft-cli convert art.svg out.dst` works with no
  further code.
- **`document.export {format: "dst" | "exp", …}`** with options: `machine` (`commercial15` default,
  or `generic`), `mode` (`run` | `manual`), `stitchLength`, `minStitch`, `maxStitch`, `locks`,
  `jumpsPerTrim`, `trimBeforeColorChange`, `start`, `returnToStart`, `name`, `selectedOnly`. An
  unknown key is refused. The reply carries the warnings and the statistics.
- **`document.open`** of a `.dst` or `.exp` file, with an optional `stitch: {jumpsPerTrim}` object.
- **`stitch.info`** (query): the statistics and warnings an export with the same options would
  give, without writing a file.
- **`stitch.calibration`**: adds the calibration design to the document as ordinary stroked paths,
  generated in code. See [Stitch-out acceptance](#stitch-out-acceptance).

Export As lists the two formats through the format table. A dedicated options dialog waits for the
sibling app's UI spec; until then the options are command parameters.

## Errors and untrusted input

- Readers treat files as hostile: bounded sizes and counts, `.get()` indexing, no panics, no
  `unsafe`. A truncated or odd file yields what could be read plus a warning, or an error message.
- Writers return `Result<Output { bytes, warnings }, String>`; the engine maps failures to
  `EngineError`.
- Float inputs (`stitchLength` and the like) are checked finite and in range before use.

## Tests

- **Unit, format crate.** Every move from −121 to +121 encodes and decodes on both axes. A
  hand-built plan gives exact header and record bytes, written out by hand in the test from the
  tables above. The same for EXP.
- **Property.** Write then read returns the prepared plan, for both formats. Arbitrary bytes never
  panic either reader and never allocate past the limits (added to `engine/tests/import_fuzz.rs`).
  After `prepare`, no stitch exceeds `max_stitch`, no jump exceeds `max_jump`, and no run of jumps
  reaches `jumps_per_trim` without a trim before it.
- **Stitch crate.** Running stitch hits corners and the path end; rounding does not drift over a
  long path; lock stitches appear where the rules say; warnings fire for needle count and field.
- **Engine.** Export, open and `stitch.info` through commands; open-then-export in `manual` mode
  reproduces the needle points; the command sweep covers the new commands.
- **Cross-check, by hand, not in CI.** Our files read with pyembroidery (installed from PyPI, used
  as a black box) give the same stitches, and its files read with ours. Recorded in
  `docs/development.md`. CI workflows are not changed.

## Stitch-out acceptance

`stitch.calibration` draws one design that answers the questions the documentation leaves open:

- a 100 mm square with 10 mm ticks: size and scale;
- a letter F in one corner: whether the design is mirrored or upside down;
- three colours: how the machine handles colour changes and needle assignment;
- two shapes 30 mm apart and two 5 mm apart: whether trims happen where the file asks;
- a design whose first stitch is off-centre: where the machine places the start.

The operator records: whether the file loads; the stitch count, colour count and size the panel
shows against `stitch.info`; measured size; orientation; where trims and stops happened; whether
lock stitches hold. The spec's unverified defaults are then corrected, and the results go into
`docs/embroidery-parity.md`.

The work is done when the automated tests pass under `cargo xtask ci` and the calibration design
stitches correctly from both a DST file and, if the machine accepts it, an EXP file.

## Machine facts

For the BAi Mirror, from reseller listings seen as search snippets only (the vendor's own sites
were not reachable): 15 needles, one head; a touch-screen controller of the vendor's own naming;
designs load from a USB stick or over WiFi; maximum field 500 × 350 mm; up to 1,200 stitches per
minute. Listings conflict on formats: one says DST only, others DST and DSB. So EXP may not load on
this machine, and DST is the format that must work.

**Not documented anywhere found**, and so tested by the calibration stitch-out: how needles are
assigned to colour changes; the jumps-per-trim setting and its default; the longest stitch
accepted; whether the header's counts and extents are checked; where the design is placed relative
to its start point.

## Open decisions

1. **Where the sibling app lives.** Recommended: a second app in this workspace
   (`apps/<name>`), sharing every crate including `ui-egui`, so nothing is duplicated and one CI
   run covers both. A separate repository would need these crates published as a dependency first.
   This spec works either way.
2. **The sibling app's name.**
3. **`ST:` and the end record.** The references disagree on whether the count includes the end
   record and expanded trim jumps. This spec counts every record written. Corrected after the
   stitch-out if the panel disagrees.

## Docs to update when this lands

`docs/embroidery-parity.md` (rows and gaps E1, E2), `docs/architecture.md` (the two crates and
their layers), `docs/development.md` (the cross-check procedure).
