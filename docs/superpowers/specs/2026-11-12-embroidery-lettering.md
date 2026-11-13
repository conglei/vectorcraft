# Embroidery lettering, first pass (gap E9, STI-23)

> **Status:** first pass implemented 2026-11-12, measured by tests and in the running app; not yet
> sewn at a machine. Numbers marked *estimated* are starting points to check at the stitch-out.
> **Change:** new spec.

Type in StitchCraft used to be skipped ("objects of other kinds were not stitched"), and its
outlines set to Satin sewed a 3 mm column *around* each glyph. This pass sews type as lettering:
satin columns that follow each letter stroke. Clean room: behaviour from public digitizing guides
and Ink/Stitch's documentation (GPL, read as documentation only, no code looked at); the
algorithms are textbook ones written here.

## Survey

- Ink/Stitch (GPL-3.0, docs only): a satin column is two rails plus optional rungs; the stitches
  alternate between the rails ("peak-to-peak" spacing); centre-walk underlay runs along the middle
  and back by its repeats; pull compensation pushes each pair of needle points outward by a fixed
  length (<https://inkstitch.org/docs/stitches/satin-column/>,
  <https://inkstitch.org/docs/satin-tools/>, <https://inkstitch.org/docs/lettering/>).
- Medial-axis crates: `spade` (MIT/Apache-2.0) gives Voronoi via Delaunay; not added. Letters are
  small, so a raster distance transform and thinning (Felzenszwalb–Huttenlocher 2012; Zhang–Suen
  1984, both published algorithms) at about 0.05 mm per pixel is cheap and needs no dependency.
  `vectorcraft-trace`'s thinning is private and in the same layer; not shared.
- Layering: `vectorcraft-stitch` now uses `vectorcraft-text` (both layer 2; `text` before `stitch`
  in the intra-layer order) to lay out type, without the bundled fonts.

## Model

- `StitchType::Lettering` (id `lettering`): type sewn as lettering. A Text node whose type is
  Automatic, Lettering or Satin is sewn as lettering; Tatami fills each glyph; Run sews the
  skeleton as running stitch; None skips it; Manual is not used for type.
- Parameters on `StitchProps` (inherited like the others): `letterTrims` (`word` default,
  `letter`, `none`), `minColumn` (mm, 0.3–3, default 1.0), `runUnder` (letter height in mm under
  which letters are running stitch, 0–20, default 5), `compensation` (mm each side, default 0.2
  for lettering, *estimated*), `underlay` (on/off; default on for satin lettering),
  `satinSpacing` (default 0.4 mm), `runLength` (lettering runs default 1.5 mm, *estimated*).
- Letter height: the tallest glyph's ink height in the text object (cap height for "Hello").

## Glyph to columns (`vectorcraft-stitch::lettering`)

1. The glyph outline (mm, holes included, nonzero rule) is rasterised at max(0.05 mm, size/800),
   so a grid is at most about 800 × 800 cells. Non-finite input is rejected; point and edge
   counts are capped.
2. An exact Euclidean distance transform gives each inside pixel's half width; Zhang–Suen
   thinning gives a one-pixel skeleton.
3. The skeleton becomes a graph: end pixels, junction clusters, and the paths between them (pure
   loops get a node of their own). Spurs shorter than 1.5 × the junction's half width are pruned
   and nodes left with two edges are merged.
4. Each edge is smoothed and resampled at 0.25 mm; free ends are extended along their tangent to
   the outline. At each centre point the normal is cast both ways to the vector outline: the hits
   are the rails, held to 0.3–1.6 × the distance-transform half width (so columns at a junction
   overlap a little and no gap shows).
5. Classification: a stretch that would sew narrower than `minColumn` (its width plus twice the
   pull compensation) becomes running stitch along the centre;
   a glyph whose widest stroke passes 8 mm becomes tatami fill (whole glyph; partial fills are
   later work); letters under `runUnder` are running stitch along the skeleton.

## Variable-width satin (`satin::rails_satin`)

Two rails of paired points; stations every half spacing along the longer rail, the other rail
taken at the same rung position, alternating sides; each needle point pushed out along its rung
by the compensation; stitches over the maximum split as `satin()` splits them. `satin()`
(constant width) is unchanged. A path or compound path of exactly two open subpaths with type
Satin is sewn this way (the digitizer's two-rail model), its rails resampled to a common count by
arc length and the second reversed when drawn the other way.

## Order inside a letter and joins

Within a glyph: a depth-first walk of the skeleton graph from the node nearest the needle. Every
edge that must be come back along is travelled out with a centre run (the underlay, hidden under
the satin) and covered by its satin on the way back; the last edge at each node of the final
path is covered going forward (after a centre run there and back when underlay is on), so the
letter ends towards the next one. No trims inside a letter; separate parts (the dot of an i) are
joined by a jump.

Between letters: the next letter starts at its skeleton node nearest the needle (closest join).
A gap under 1.2 mm is sewn as a stitch; otherwise a jump, with a trim where `letterTrims` says
(`word`: between words and lines; `letter`: between all letters; `none`: never). The text object
is one sewing-order item, sewn forwards only.

## Thresholds and sources

| Rule | Value | Sources |
|---|---|---|
| Run stitch letters under | 5 mm letter height | mixed case min 5 mm, all caps 3.8 mm (<https://www.piworld.com/embroidery-digitizing-tips-on-creating-designs-with-letters/>); all caps min 5 mm (<https://blog.ignitiondrawing.com/small-lettering-guide-part-1/>) |
| Centre-run underlay | on for satin letters (6–10 mm the common case) | centre walk for small letters, never edge walk (<https://members.asicentral.com/news/how-to/may-2018/keep-your-small-letters-crisp/>); 6–10 mm uses centre run (<https://hoopingstation.com/blogs/articles/wilcom-hatch-lettering-underlay-when-to-trust-auto-underlay-and-when-to-override-it>) |
| Minimum column | 1.0 mm | not narrower than the needle, 1 mm example (ASI, above); 0.8 mm (piworld, above); 1.0 mm (<https://graphics-pro.com/?p=95539>) |
| Widest satin | 8 mm, wider becomes fill | 8–10 mm (Graphics Pro, above; <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Large_Lettering.htm>) |
| No trim under | 1.2 mm gap | piworld, above |
| Pull compensation | 0.2 mm each side, *estimated* | no universal value (<https://wilcom.com/resources/blog/push-and-pull-compensation>); a fixed outward push as in Ink/Stitch's satin docs |
| Satin spacing | 0.4 mm | "0.4mm is normal" (<https://embrilliance.com/Help/Platform%20Win%201158/stitchproperties.htm>) |

## Commands

- `text.toStitches` (Embroidery › Convert Type to Stitch Objects): selected type becomes a group
  per text object holding a group per letter: each column a compound path of two open rails
  (Satin, with its spacing, compensation and underlay), runs as open paths (Run), filled glyphs as
  closed paths (Tatami). One undo step. The objects are sewn one by one, so they are joined by
  trims where the live type travels; Closest Join and the sewing order apply to them.
- `stitch.lettering.info {ids?, options?}`: per type object, what it sews as: letter height, mode
  (satin, run, fill), columns, runs, fills, trims, stitches (as `stitch.info` counts them for
  that object alone).
- The parameters are set with `stitch.setParams` (`minColumn`, `runUnder`, `letterTrims`,
  `compensation`, `underlay`, `satinSpacing`, `runLength`) and `stitch.setType` (`lettering`) on
  type, groups or layers; `stitch.props` and the Stitch Properties panel show them.
- MCP: not added in this pass; `stitch.lettering.info` and `text.toStitches` are reachable
  through the generic command tool.

## Measured (2026-11-12, StitchCraft dev build, Source Sans 3 Regular)

| Text | Mode | Stitches | Columns | Trims |
|---|---|---|---|---|
| "Hello" 15 mm (cap 10.8 mm) | satin | 652 | 46 | 0 |
| "Hello you" 15 mm | satin | 1118 | 89 | 1 |
| "Test" 12 mm (cap 7.9 mm) | satin | 465 | 35 | 0 |
| "Test" 5 mm (cap 3.3 mm) | run | 53 | 0 | 0 |

`stitch.lettering.info` for "Hello" took 0.07 s (planning included). Sizes are font sizes.

## Later

Partial fills inside a glyph, edge-walk and zigzag underlay for tall letters, bottom join, script
fonts' connecting strokes, monograms, team names, an Ink/Stitch font reader, pre-digitized fonts.
