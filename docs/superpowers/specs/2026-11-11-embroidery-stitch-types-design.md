# Embroidery stitch types, first pass (tatami fill and satin column): design

> **Status:** draft for owner review · **Date:** 2026-11-11 · **Covers:** the first part of gap E3
> in [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-10) · **Builds on:**
> [the stitch core](2026-11-11-embroidery-stitch-core-design.md)

## Goal

A filled shape exports as a tatami fill and a stroked path as running stitch or a satin column, so
ordinary vector art stitches without per-object stitch settings. This pass adds two generators to
`vectorcraft-stitch` and two export options. Stitch properties stored on objects are E4.

## Sources consulted

| What | URL | Licence | Taken |
|---|---|---|---|
| Ink/Stitch manual, fill stitch | <https://inkstitch.org/docs/stitches/fill-stitch/> | documentation, read only | The parameter set: row spacing, stitch length along a row, angle (degrees, counter-clockwise, negative allowed), rows staggered so penetrations repeat only every few rows, travel between sections inside the shape rather than across it |
| Ink/Stitch manual, satin column | <https://inkstitch.org/docs/stitches/satin-column/> | documentation, read only | Zig-zag spacing measured peak to peak; stitches longer than a maximum are split |
| Roidery README | <https://github.com/lork-fork/Roidery> | MIT OR Apache-2.0 | Nothing beyond confirming that tatami and centre-line satin are the usual first stitch types. Its source was not read: its provenance is unverified |
| OpenStitch README | <https://github.com/cristianonescu/OpenStitch> | MIT OR Apache-2.0 | Confirmation that a row phase (stagger) and travel along the outline are usual. Its source, and its `compat/` folder in particular, were not read |

No GPL or AGPL source (Ink/Stitch, Inkscape, PEmbroider) was opened. No code, constant or string
was copied from any of the above, so nothing is added to `NOTICE`. The Ink/Stitch manual pages do
not show default values, so the defaults below are this project's own estimates, to be tuned at the
stitch-out.

## Parameters and defaults

| Option | Default | Range | Source of the default |
|---|---|---|---|
| `fill.type` | `tatami` | `tatami`, `none` | Owner's scope (STI-10): fills are sewn by default |
| `fill.angle` | 0° | −360–360 | Horizontal rows; estimated |
| `fill.spacing` | 0.4 mm | 0.2–5 | The usual tatami density, as the parity doc's "density" (estimated) |
| `fill.stitchLength` | 3 mm | 1–8 | Estimated: long enough to lie flat, short enough not to snag |
| `fill.stagger` | 0.25 | 0–1 | A quarter stitch per row: penetrations repeat every four rows (estimated) |
| `stroke.type` | `run` | `run`, `satin` | The core spec's behaviour, unchanged |
| `stroke.spacing` | 0.4 mm | 0.2–5 | Peak to peak, as the Ink/Stitch manual measures it (value estimated) |
| `stroke.width` | the stroke weight | 0.5–20 mm | What the art shows; a stroke weight outside 0.5–20 mm is held to it, with a warning |

The machine's maximum stitch (`maxStitch`, the profile's by default) bounds both generators.

## Tatami fill (`crates/stitch/src/fill.rs`)

1. The path's subpaths are flattened (0.05 mm, as for running stitch) into closed rings in mm; an
   open subpath closes itself, as a fill does on screen.
2. The rings are turned by the angle so rows run along `u`. Rows sit at `v = k · spacing` for whole
   `k`: the grid is global, so neighbouring shapes at the same settings line up.
3. Each row is cut where it crosses the rings (half-open, so a vertex counts once). Crossings are
   sorted along the row and wound by edge direction; the path's fill rule (non-zero or even-odd)
   decides which stretches are inside. So holes are kept, and a compound path fills as it draws.
4. Needle points on a row are at `phase + m · stitchLength`, where `phase` is
   `(k · stagger mod 1) · stitchLength`, plus the two ends; points closer than a quarter stitch to
   an end are left out.
5. Rows are sewn back and forth. From a span the needle goes to the nearest end of an unsewn span
   on the next row that overlaps it, if the straight connector crosses no edge and its middle is
   inside. Otherwise the section ends: each section is filled continuously.
6. The next section starts at the first unsewn span in row order, at its end nearest the needle.
   When the needle and that end lie on the same ring, the needle travels along the ring the shorter
   way, each vertex moved inward by half the spacing when that point is inside (an inset edge run).
   Otherwise a straight run if it stays inside, otherwise a new run (the plan trims and jumps).
7. Every stitch longer than the maximum (less 0.15 mm for rounding) is split evenly; connectors and
   travel are stepped at the stitch length.

Limits: 100,000 rows ("the fill has too many rows…"), 20,000,000 edge-band entries, and the plan's
2,000,000-step budget, all errors before the memory is used.

## Satin column (`crates/stitch/src/satin.rs`)

1. The centre line is the flattened subpath. The rails are the centre line moved ±width/2 along its
   normals, mitred at corners, the mitre held to twice the half width. `pathops::offset_path`
   returns one closed outline, not two paired rails, so the rails are built from the normals.
2. Stations sit evenly along the centre line, `spacing / 2` apart, the first and last at its ends
   (square ends). They alternate rails, so each rail is pierced every `spacing`.
3. A stitch longer than the maximum is split into equal parts; every other stitch's split points
   move half a part, so they do not line up down the column (auto split).

## How a path's stitch type is chosen

- A visible fill becomes a tatami fill (unless `fill.type` is `none`), sewn first, in the fill's
  colour (black for a gradient or pattern, with a warning). Then a visible stroke becomes running
  stitch or a satin column by `stroke.type`, in the stroke's colour.
- Art on the layer an opened stitch file made keeps the core spec's per-layer rules: its fills are
  not sewn and satin does not apply, so its needle points export unchanged.
- `mode: manual` still applies to running-stitch strokes; satin strokes are satin.
- The error for an empty plan now reads "nothing to stitch: the document has no stroked or filled
  paths".

## Tests

- Fill, unit: a rectangle is one run with rows 0.4 mm apart and alternate directions; penetrations
  differ from row to row; a ring (both fill rules) leaves the hole empty, and a hole wound the same
  way fills under non-zero; a U shape is one run whose stitches never cross the gap; 90° turns the
  rows; row, point and settings limits are errors.
- Fill, property: a rectangle with a rectangular hole at any angle, spacing, stitch length, stagger
  and maximum: every needle point inside the shape grown by the spacing, no sampled point of any
  stitch in the hole, no stitch past the maximum; on a rectangle, rows at the requested spacing.
- Satin, unit: a straight column alternates rails with square ends; a wide column is split with
  staggered split points; a curved column stays between its rails; closed and degenerate lines.
- Satin, property: along any arc, needle points within the rails and no stitch past the maximum.
- Plan: a filled ring outlined in satin at any angle and size has no stitch past the machine's
  maximum, and `prepare` warns of nothing.
- Engine: a filled rectangle and a stroked circle export to DST and read back the expected stitch
  counts; the options change the result as documented; bad and unknown option keys are refused.

## Deferred

Underlay; pull compensation; triple, back and stem running stitch; zigzag and E stitch; lock stitch
rules; trims by connector length; per-object stitch properties and stitch angle lines (E4); variable
satin width from the width profile; rail pairing for filled shapes (satin from a fill); shortening
on the inside of sharp turns, and rails that cross where a curve is tighter than half the width;
travel hidden under the fill still to be sewn (now: along the edge); an optimal section order.
