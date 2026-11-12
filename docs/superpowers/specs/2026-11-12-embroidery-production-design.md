# Embroidery production (gap E7, STI-16)

> **Status:** first pass implemented 2026-11-12, measured by tests; not yet used at a machine.
> **Change:** new spec.

What an operator needs between a finished design and the machine: the hoop, a check for problems,
the estimates, a printable worksheet and one folder holding it all. Clean room: behaviour from
public descriptions of commercial packages (see [embroidery-parity.md](../../embroidery-parity.md)
Sources); hoop data, worksheet layout and rules are original.

## Hoops

- Model, in `vectorcraft-doc` (`stitch_design.rs`, below the stitch crate):
  `StitchDesign::hoop: Option<Hoop>`, `Hoop { id: Option<String> (library id, None custom), name,
  width_mm, height_mm, shape: HoopShape { Rectangle, Round }, cap }`, camelCase, read leniently
  (a damaged hoop reads as unusable: `StitchDesign::hoop()` filters it). `Hoop::holds(box, margin)`
  is the geometry: a rectangle holds the box inside its sides less the margin; a round hoop holds
  it when every corner lies inside the ellipse shrunk by the margin.
- Library, in `vectorcraft-stitch::hoop`: ten generic sizes by inner dimensions (100 × 100, round
  150, 130 × 180, 200 × 200, round 240, 300 × 200, 360 × 200, 400 × 300, a 500 × 350 tubular frame
  and a 360 × 80 cap frame). No maker's names or part numbers. `library()`, `find(id)`,
  `fits(design_mm, hoop, margin)`, `fit(design_mm, margin, field_mm)` (the smallest by area, cap
  frames left out, within the machine field), `custom(...)` (sides 10–2000 mm).
- Placement: the hoop's centre on the page is the first artboard's centre. `stitch.hoop.fit`
  reports `offsetMm`, the stitches' centre from it, so the canvas (STI-31) can draw the hoop the
  same way. `stitch.hoop.center` moves the art there (one undo step through `apply_transform`).
  A DST written with `start: center` has its origin at the design's centre, so the position on the
  page matters to the machine only when a design start point is set; the check reports an
  off-centre design as info.
- Commands: `stitch.hoops.list {machine?}`, `stitch.hoop.get`, `stitch.hoop.set {id | widthMm,
  heightMm, name?, shape?, clear?}`, `stitch.hoop.fit {marginMm?, …plan options}`,
  `stitch.hoop.center {…plan options}`.

## Estimates (in `stitch.info`)

- `needle` per thread: `assign_needles` on the machine's needle count (first colour sewn on needle
  1, and so on).
- `topMm`: the stitched length of all threads. `bobbinMm = 0.5 × topMm`: thread use splits about
  2/3 top and 1/3 bobbin (Madeira,
  <https://www.madeira.com/en-in/embroidery-supplies/industrial-embroidery-threads>; per-1,000-stitch
  figures from thread makers give a similar half). Stitched length understates real consumption
  (the thread also goes into the fabric); the figures are labelled estimates.
- `runTimeS = (stitches + jumps) / spm × 60 + colorChangeS × colour changes + trimS × trims`.
  `spm` defaults to the profile's `max_spm` (1,200 on `bai_mirror`), else 800. `colorChangeS` 5 s
  and `trimS` 3 s are assumptions to measure at the stitch-out.

## Design checker (`stitch.check`)

Rules on the prepared plan (`vectorcraft-stitch::check_plan`) and the hoop (engine):

| Code | Level | Rule (defaults are assumptions to tune at the stitch-out) |
|---|---|---|
| `empty` | error | no stitches (also when the planner finds nothing to sew) |
| `overMaxStitch` | error | a stitch longer than the machine maximum on an axis |
| `shortStitches` | warning | stitches shorter than `shortMm` (0.3, the export's default minimum: the export drops shorter ones, so these come from a lower `minStitch` or an opened file; 0.5 flagged the normal ends of tatami rows) |
| `longStitches` | warning | stitches longer than `longMm` (7) |
| `density` | warning | 2 mm cells with more than 4 needle points per mm² (`densityCellMm`, `maxDensity`) |
| `trims` | warning | more trims than `maxTrims` (50) |
| `colors` | warning / info | more colours than needles; else the count |
| `fieldTooSmall`, `hoopTooSmall`, `noHoopFits` | error | the design does not fit the field, the chosen hoop, or any hoop |
| `hoop`, `offCenter` | info | the hoop it fits; over the hoop's edge where it sits now |

`where` is document units (the first case, or the densest cell's centre). `stitch.info` adds the
warning and error messages to its `warnings`.

## Worksheet (`stitch.worksheet {path, hoop?, …}`)

Built as an A4 document: point type in the bundled Source Sans 3 (so it renders with
`CRAFT_FONTS` empty), rectangles, and the realistic stitch preview (`vectorcraft-stitch::preview`,
about 1000 px on its long side) as an embedded PNG image, written by the PDF export. Contents:
design name, date (`chrono`, from `output_time`), machine; the picture; size, stitches, colours,
changes and stops, trims, jumps, hoop, machine, speed, run time, top thread, bobbin, design-check
counts; the stop table (number, swatch, code, thread name, needle, stitches, length), one row per
colour block, cut with "… and N more stops" when it would run past the page; a note on how the
estimates are made. No barcode yet (Code 128 was optional).

## Production folder (`stitch.exportProduction {dir, name?, formats?, worksheet?, …}`)

Everything is encoded before anything is written. `dir` is made if missing and must be a folder;
`name` (default the design name) may hold letters, digits, spaces, `-`, `_` and `.`, not start
with a dot and not contain `..`, so files cannot land outside `dir`. Formats default to the
machine profile's panel formats (DST and DSB on `bai_mirror`), else DST. Files of the same names
are replaced. Returns the files with their sizes, the checker's findings and the files' warnings.

## Surfaces

- Menu: Embroidery › Center Design in Hoop, Production Worksheet…, Export for Production…; Stitch
  Info shows the estimates, the needle per thread, a hoop dropdown and a Design Check section.
- MCP: `stitch_worksheet`, `export_production` (other commands through `run_command`).
- Plug-ins: the hoop and check commands are allowed; the worksheet and the folder export write
  files and are refused by the file rules.
- The canvas does not draw the hoop yet: STI-31 owns the canvas and reads this model.
