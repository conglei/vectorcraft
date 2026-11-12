# Photo stitch, second pass (E11, STI-30)

Status: implemented 2026-11-12, measured by tests on the picture drawn in code
(`photo.syntheticFur`), not on real photos and not stitched on a machine.
First pass: [2026-11-11-embroidery-photo-stitch-design.md](2026-11-11-embroidery-photo-stitch-design.md).

## Problem

The first pass traced each quantized photo colour as its own region. Several photo colours that
match the same chart thread were sewn as separate regions, with a trim and a jump between every run.
On the 480 × 360 synthetic fur picture at 100 mm it made 33 regions (the brown half alone 31), 9,178
stitches and 257 trims; neighbouring regions of one thread left visible gaps. Eyes and noses were
lost in the long rows, a photo could only be sewn in colour, and the app had no dialog for it.

## Approach

1. **Merge by thread at the label level** (`crates/engine/src/cmd/photo/labels.rs`). The softened
   photo is smoothed and quantized as the tracer does (`vectorcraft_trace::smoothed` when `continuous_tone`, `quantize`,
   `denoise` by `minArea`). Each photo colour is matched to its chart thread and every pixel is
   relabelled by thread, so photo colours of one thread become one area. A bounded one-dimensional
   morphological closing (Serra, *Image Analysis and Mathematical Morphology*, 1982) closes runs of
   another thread no wider than `mergeGapMm`, along rows and then columns. Areas under
   `minRegionMm2` are absorbed into the neighbour they share the longest border with (the existing
   `denoise`). The cleaned labels are painted in their threads' colours and traced once more, so
   outlines are fitted curves as before. Every step is a fixed number of passes over the pixels.
2. **Fewer trims** (`join_runs` in `stitches.rs`). Within a colour the runs are put in nearest-first
   order (each may be reversed; up to 6,000 runs). A gap of at most `travelMm` becomes travel: a
   stitched straight line when the way stays inside the colour's regions (later rows cover it),
   else one jump record, which a machine never reads as a trim. Longer gaps keep a trim and a jump.
3. **Eye and nose detail pass** (`detail.rs`). Connected pixels whose lightness differs by at least
   60 levels from the mean of a box around them (radius 12% of the short side; summed-area tables)
   are candidate features. Kept: at least 1 mm², at most 5% of the opaque area, compact (box aspect
   at least 0.35, at least 40% filled), at most 16, largest first. Each is sewn after the fills in
   the chart thread nearest its mean colour: short fill rows at 45° (0.35 mm spacing, 1–2 mm
   stitches) and a running stitch round its outline. Light and dark features are found alike: the
   synthetic picture's eye is light; a pet's eye or nose is dark.
4. **Single-colour photo modes** (`mono.rs`), one thread: `color` matched to the chart, else the
   chart's darkest thread. Tone is the lightness after `blur`, stretched between its 2nd and 98th
   percentiles.
   - `fill: rows`: rows `spacing` apart, sewn back and forth. Each row zigzags about its line with
     an amplitude up to 0.45 × spacing growing with darkness (so rows never touch), and steps from
     `maxLength` (light) down to half the spacing (dark): amplitude-modulated line shading, as in
     engraving and plotter "squiggle" drawings. Commercial digitizing packages describe similar
     single-colour photo fills of rows whose spacing follows tone; only the behaviour is taken.
   - `fill: meander`: one open path through the cell centres of an adaptive Hilbert curve (D.
     Hilbert, "Über die stetige Abbildung einer Linie auf ein Flächenstück", *Math. Ann.* 38, 1891).
     The picture is divided like a quadtree, a cell split while it is larger than
     `spacing + (maxLength − spacing) × (1 − darkness)²`; cells are visited in Hilbert order. Each
     sub-curve starts in the corner cell at its origin and ends in the corner cell along its first
     axis, so consecutive leaves always share an edge whatever their sizes; each stitch then lies
     within the two cells it joins and no two stitches cross. Transparent cells break the path
     (joined by travel, or a trim when far).
   Ink/Stitch's meander fill ([docs](https://inkstitch.org/docs/stitches/meander-fill)) tiles
   quilting patterns over a shape; its documentation was read for context only (GPL: no code).
   A survey found no permissively licensed adaptive Hilbert implementation to port; the code here
   is written from the definition above.
5. **App dialog**: Embroidery › Photo to Stitches… (`crates/ui-egui/src/dialogs/photo_stitch.rs`).
   It holds the command's params; OK runs `photo.toStitches` and shows the new document; an error
   stays in the dialog.

## Params added to `photo.toStitches`

| Param | Default | Range | Meaning |
|---|---|---|---|
| `fill` | `field` | field, tatami, rows, meander | rows and meander are new |
| `mergeGapMm` | 0.5 | 0–3 | gaps of another thread closed between areas of one thread |
| `minRegionMm2` | 2 | 0–10000 | smaller sewn areas are absorbed into a neighbour |
| `travelMm` | 3 | 0–12 | longest gap in a colour joined without a trim |
| `features` | true | bool | the eye and nose detail pass (field fill only) |
| `color` | darkest thread | `#rrggbb` | thread of rows and meander |
| `spacing` | 0.4 (field, tatami), 1 (rows, meander) | 0.2–5 | rows apart, or the meander's finest cell |

`photo.regions` takes `mergeGapMm` and `minRegionMm2` too. Both report `regionsBefore`, `merged`
and `absorbed`; `photo.toStitches` also reports `blocks` (thread, stitches, jumps, trims per colour
block) and `details` (centre and radius in mm per feature).

`detail` keeps its first-pass meaning (how closely outlines follow the photo), so the detail pass
is switched by `features`.

## Acceptance (measured by tests)

- Same-thread colours become one area; slivers close; specks are absorbed (`labels` unit tests).
- On the fur picture each thread is at most two regions, `merged > 0`, and the default `travelMm`
  has under half the trims of `travelMm: 0` without costing 10% more stitches (`stitches_tests.rs`).

Measured with the release CLI on the 480 × 360 picture at 100 mm, `provider: none` (first pass →
second pass): regions 33 → 4 (brown 31 → 2; 29 merged, 0 absorbed), stitches 9,178 → 8,057, trims
257 → 16 (3, 8 and 5 per colour block), jumps 290 → 61, colour changes 2 → 2. Without the detail
pass: 7,558 stitches and 13 trims. Rows: 5,366 stitches, 0 trims; meander: 3,973 stitches, 0 trims.
- The detail stitches lie within the eye's circle plus 0.5 mm, and `features: false` sews none.
- The meander is one path for an opaque picture, never crosses itself, and is denser where darker;
  rows zigzag wider with shorter stitches where darker; transparency breaks both.
- The dialog opens from the Embroidery menu, runs the command and opens the document, and shows an
  error without closing (headless egui frame test).

## Deferred

- Layered fills (a lighter layer over a darker one), underlay under the direction fill, pull
  compensation.
- Satin for the detail pass; features found by a model rather than by contrast.
- Rows at an angle or following the direction field; meander on very long thin pictures uses
  stretched cells.
- Tuning on real pet photos and a stitch-out.
