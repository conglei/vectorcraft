# Embroidery stitch preview: design

> **Status:** first pass implemented · **Date:** 2026-11-11 · **Covers:** the realistic preview, the needle point, jump and function overlays and the density map of gap E5 in [embroidery-parity.md](../../embroidery-parity.md)

## Goal

See what a DST or EXP export would sew before stitching it: every stitch as lit thread on fabric,
optionally with jumps, needle points and machine functions marked, or a density map that shows
where thread piles up. Agents and the CLI get it as a PNG; the app UI comes later.

## Placement

The renderer is `crates/stitch/src/preview.rs` (`vectorcraft-stitch`, L2). It turns a
`StitchPlan` and `PreviewOptions` into opaque RGBA pixels and uses nothing but `std`:

- The plan model lives in this crate, so no new crate and no layer table entry are needed.
- `crates/render` (L3) draws documents through `vello_cpu`; a preview is not a document, and
  turning two million stitches into document paths would cost far more memory than the picture.
- A purpose-built capsule rasterizer is about 150 lines, builds for `wasm32`, and adds no
  dependency. The engine encodes the PNG with the `image` crate it already uses.

`stitch.preview` in `crates/engine/src/cmd/stitch.rs` builds the plan with
`fileio::stitchfile::plan`, the same code `document.export {format: dst|exp}` and `stitch.info`
use, so the preview shows exactly the prepared plan an export writes (lock stitches, split long
stitches, travel and trims included).

## Rendering model

- **Picture.** The extents of every stitch and jump plus a 2 mm margin, at `scale` pixels per mm
  (default 10, 0.5–50). More than 8192 pixels on a side is an error that names the design's size.
  The buffer is allocated with `try_reserve_exact`, so too little memory is an error, not an abort.
  Besides the picture only the density grid (at most 2048 × 2048 cells) is allocated; the plan is
  streamed, so a 2,000,000-step plan costs nothing extra.
- **Sewn thread.** Consecutive `Stitch` steps of a block are sewn thread. A jump, trim, stop or
  colour change breaks the run: floats left by untrimmed jumps are shown only by `showJumps`.
- **A stitch** is a capsule (a segment with round ends) of radius `threadWidth / 2` (default 0.4 mm
  thread). For each pixel centre near it, the closest point on the centre line gives an offset
  `n = (pixel − closest) / r`. Coverage is `clamp(r + 0.5 − |pixel − closest|, 0, 1)`, a one-pixel
  linear ramp that anti-aliases the edge and gives the exact area for straight edges.
- **Shading.** The thread is a cylinder seen from above: its normal is `(n.x, n.y, sqrt(1 − |n|²))`.
  Light comes from the top left and front (`L = (−1, −1, 2)/√6`). Colour = thread × `(0.3 + 0.8 ·
  max(N·L, 0))` + a Blinn–Phong highlight `0.35 · max(N·H, 0)^24`, all × a "sink" factor that falls
  from 1 to 0.7 within 1.5 radii of either needle point, where the thread goes into the fabric. The
  round caps use the same formula, so ends look rounded. This is the two-dimensional form of the
  lit thin cylinders of Chen et al. (below).
- **Edge shadow.** Before each stitch a soft shadow (radius 1.15 r, offset down-right by
  (0.15 r, 0.3 r), opacity 0.22 falling to 0 at its edge) is drawn, so a stitch reads as lying on
  the ones below. Later stitches draw over earlier ones.
- **Fabric.** A flat colour, default `#ece8de` (light warm neutral), parameter `fabric`.

### Overlays (drawn last, over either view)

| Option | Mark |
|---|---|
| `showJumps` | each jump as a dashed line (4 px dash, 3 px gap, 1.2 px wide, dark grey) from the previous position; a jump over 70,000 px is drawn solid |
| `showNeedles` | a white dot with a dark rim at each needle point, about a third of the thread width |
| `showFunctions` | rings at the current position: red at a trim, orange (larger) at a stop, a disc in the new thread's colour with a dark rim at a colour change. Ring size 1.2 mm, kept 4–24 px |

### Density view

`view: density` replaces the thread with a grid of 1 mm cells (larger only when a design needs
more than 2048 cells on a side). Each stitch is cut into pieces of at most half a cell, and each
piece adds its length to the cell holding its middle. A cell's value is its **coverage**:
stitched length × thread width / cell area, so a fill whose rows are one thread width apart has
coverage 1. The fixed ramp: pale green near 0, green at 1, yellow at 1.5, red at **2 and above,
the documented "too dense" threshold** (`TOO_DENSE`: thread laid about twice over the area, as
where two fills overlap). The command returns `maxCoverage` and `denseCells` (cells at or above 2)
so an agent can check density without looking at the picture. No legend is drawn.

## Command

`stitch.preview` (query, so it never changes the document or its history) takes every plan option
of `stitch.info`, plus `view`, `scale`, `threadWidth`, `fabric`, `showJumps`, `showNeedles`,
`showFunctions` and `path`. With `path` it writes the PNG there (through the automation file
roots, and not available on the web) and returns `{width, height, path, warnings}`; without it,
`{width, height, pngBase64, warnings}`, at most 2048 × 2048 pixels inline (larger is an error that
asks for `path` or a lower scale). `warnings` are the plan's (`stitch.info`'s without the file
writer's, since no file is written). Plug-ins cannot call it, as for every command that names a path.

## Tests

- `vectorcraft-stitch` (`preview::tests`): capsule coverage symmetric about both axes; covered
  area of a straight and a diagonal stitch within 3% of `2rL + πr²`; drawing far outside the
  picture, and with NaN and infinite ends, stays inside the buffer; size = extents + margin; bad
  options, an empty plan and a design over 8192 px are errors; later stitches cover earlier ones;
  each overlay draws only when asked; two coincident runs give twice the coverage of one; density
  mode counts too-dense cells; a 2,000,000-step plan renders.
- Engine (`cmd::stitch::tests`): one stroked rectangle previews at its stitch extents plus the
  margin, with thread and fabric where expected; the calibration design at scale 2 with function
  markers matches the golden `crates/engine/tests/fixtures/stitch/calibration-preview.png` to
  within 6 per channel on every pixel (regenerate with `VECTORCRAFT_BLESS=1` after a deliberate
  change, and look at it); the density view's figures; writing to a path; refused options and an
  inline picture over the cap. The command sweep (`engine/tests/command_sweep.rs`) runs it with
  junk parameters, and the stitch file fuzz test (`engine/tests/import_fuzz.rs`) previews every
  garbage or mutated DST and EXP file that opens, in both views with every overlay.

## Prior art consulted

Ideas only; no code was copied.

- Ink/Stitch documentation, *Visualize and export* (<https://inkstitch.org/docs/visualize/>), for
  which views a digitizer expects (simulator toggles for needle points, jumps, trims, stops and
  colour changes; realistic and simple previews; a density map with red, yellow and green marks).
  Documentation read; Ink/Stitch's source is GPL and was not read.
- rustitch (MIT, <https://docs.rs/crate/rustitch/latest>): renders stitch thumbnails as
  anti-aliased line segments with tiny-skia. README read; it has no thread shading.
- Chen, McCool, Kitamoto, *Embroidery modeling and rendering in real time*, SIGGRAPH 2011 poster
  (<https://cgl.uwaterloo.ca/poster/xinling_SIGGRAPH11.pdf>): stitches as thin lit cylinders with
  Phong/Blinn lighting; the per-pixel cross-section shading here is its 2D form.
- libembroidery (zlib, Embroidermodder): a file library; no rendering code was found to learn from.
- Roidery (MIT OR Apache-2.0, `emb-mockup/src/shade.rs`, helix plies with Kajiya–Kay sheen) was
  reported by the coordinator; it was not read for this pass and nothing is taken from it.
- No permissively licensed thread shader was found on Shadertoy or GitHub.

## Deferred

The animated simulator panel and travel-through-the-design controls (step by stitch, object,
colour or function); view toggles and a preview panel in the app UI; GPU rendering; fabric
textures; per-thread sheen (plies, Kajiya–Kay); a density legend; density by stitch count within a
radius as an alternative measure.
