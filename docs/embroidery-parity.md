# Embroidery parity

> **Last reviewed:** 2026-11-11 · **Last updated:** 2026-11-11 · **Change:** minor (execution tracking: one Linear issue per gap, with blocking relations, in the StitchCraft team; `stitch.preview`: realistic thread preview, jump, needle point and function overlays, density map) · **Target:** the entry and middle levels of commercial embroidery digitizing software, on commercial multi-needle machines

What commercial embroidery software does, against what VectorCraft has today. This is a planning
checklist for the embroidery subsystem, now being built: photo or artwork in, a stitch plan and a machine file
out. It is not yet part of [target-app-parity.md](target-app-parity.md) or [gaps.md](gaps.md), and
no roadmap number moves because of it.

**Where the work is tracked.** Execution lives in the owner's Linear workspace: team StitchCraft
(key `STI`), project "StitchCraft: embroidery" (`P-STI-31`). It holds one issue per gap E1 to E16,
named with the gap id, with blocking relations that mirror the "Depends on" column below, plus the
owner decisions the gaps wait on and the calibration stitch-out. Linear is the source of truth for
who does what and in which order; this document stays the source of truth for behaviour, status
and evidence, and is updated when an issue lands. Agents without Linear access can still work from
the gap table here; the issue descriptions repeat its contents with file references.

## Summary

**12 of 41 basic-tier rows done (5) or partial (7), measured by automated tests; roughly 12% of the
basic tier's hours (estimated).** VectorCraft has a stitch plan model, machine profiles, running and
manual stitches along stroked paths, a small-stitch filter, fixed lock stitches, and DST and EXP
readers and writers, reached through `document.export`, `document.open`, `stitch.info` and
`stitch.calibration`, and a PNG preview of the export's stitches (`stitch.preview`: lit thread or a
density map, with optional jump, needle point and function marks). Fuzz tests feed garbage and mutated DST and EXP files to both readers. Files
written by VectorCraft and read by an independent implementation (pyembroidery, by hand, see
[development.md](development.md#checking-stitch-files-against-another-reader)) give the same needle
points, and the reverse. **No file has been stitched on a machine yet**: the jumps-per-trim default,
the `ST:` count, the start position and the lock stitch length are unverified until the
calibration stitch-out. There are no fills, satin, simulator, preview panel or thread charts. VectorCraft also has
most of the drawing side that a digitizer needs (shapes, pen, booleans, offsets, Image Trace, type,
a renderer, a command system, a CLI and MCP), which commercial packages either build themselves or
borrow from a bundled drawing program.

- **Basic tier** (what every package ships at its entry level, plus plain manual digitizing):
  roughly **340–570 h** remaining (estimated; 390–650 h before E1, the DST and EXP part of E2 and
  the first preview pass of E5),
  gaps E2–E10.
- **The product goal** (vivid pet portraits from a photo, with an image model preparing the photo):
  roughly **110–190 h** more (estimated), gaps E11–E12. No commercial package automates fur
  direction, so this part has no parity target; it is where the product differs.
- **Professional tier** (decorative fills, stitch-file recognition, specialty work): roughly
  **140–250 h** (estimated), gaps E13–E16, not counting attachment devices.

All hours are estimates made before any design work. Treat them as ordering information.

## Scope and decisions so far

Agreed with the project owner on 2026-11-10 and 2026-11-11:

- A full embroidery package, not only a photo converter: the basic functions of commercial software
  are in scope.
- Both an automatic pipeline (photo in, machine file out) and an interactive app feature, driven by
  the same engine commands.
- Commercial multi-needle machines first: DST and EXP before the home-machine formats.
- An image generation model prepares the uploaded photo. Faithful preparation is the default and
  restyling is an opt-in choice of style. It is a hosted API behind a provider interface, with the
  key on the machine. The pipeline must still work with no model.
- Real stitch-outs are available, so they are part of acceptance.

## Sources and confidence

Behaviour comes only from public documentation, cited in [Sources](#sources). Read this before
relying on any number here.

- **Read in full:** the Ink/Stitch user manual, the PEmbroider README and API notes, the
  pyembroidery README, the libpes wiki.
- **Search snippets only:** everything about Wilcom EmbroideryStudio, Hatch, Bernina, Tajima DG by
  Pulse, Melco DesignShop, Ricoma Chroma, Brother PE-Design, Embird, Embrilliance, mySewnet,
  Floriani, and all machine format details. The vendor sites and the format wikis were outside the
  research session's network allowlist. The feature lists agree across vendors and are safe to plan
  against. Parameter values, level charts and byte-level format facts are **unverified** and must be
  checked against the public write-ups before code depends on them.
- **Clean room.** Ink/Stitch is GPL-3.0. PEmbroider is GPL-3.0 with a second licence that bars
  commercial embroidery software. We learn from their documentation and never from their source.
  No GPL source was opened for this survey. Machine formats have no official vendor specification;
  the public descriptions are community write-ups and MIT or zlib licensed libraries.
- **Thread colours.** No manufacturer publishes an openly licensed colour table, and the tables
  bundled with GPL software cannot be copied. See E6.

## What "basic" means

The entry level of essentially every commercial package has the same set:

1. Open, convert and save the common machine formats, with batch conversion.
2. A realistic preview, a stitch-by-stitch simulator and a density view.
3. Move, rotate, mirror and scale, with stitches recalculated so density is kept.
4. Colour sequence editing, matching to a thread chart, colour sort.
5. Keyboard lettering with 130–250 pre-digitized fonts, on a line, arc or shape; kerning; monograms.
6. Automatic underlay and density from a fabric preset.
7. Trims and lock stitches, including the DST convention of several jumps for a trim.
8. Hoop or frame choice with a fit check.
9. A design checker and a small-stitch filter.
10. A printable production worksheet with the stop table.
11. Combining designs, alignment.
12. Transfer to the machine by USB stick or network.

Manual digitizing (run, satin column, fill with holes and stitch angles), node editing, appliqué and
photo conversion come one level up. Branching, curved and patterned fills, fur and feathered edges,
custom fonts, team names and device work (sequins, chenille, rhinestones) are top level or paid
add-ons. This document treats the entry set plus plain manual digitizing as the basic tier.

## Feature tables

Status is **missing** unless a row says otherwise. **Done** and **partial** are measured by
automated tests, not by a stitch-out. "Have" names what VectorCraft already has to
build on, from the code. Tier: **B** basic, **M** middle, **P** professional.

### Stitch engine and quality

| Feature | Commercial behaviour | Tier | VectorCraft today | Gap |
|---|---|---|---|---|
| Stitch plan model | Ordered stitches in 0.1 mm units with jump, trim, stop and colour change functions, grouped by object and colour | B | **done**: `vectorcraft-stitch` plan in 0.1 mm units with stitch, jump, trim and stop steps in colour blocks, capped at 2,000,000 steps; grouping by object waits for stitch objects (E4) | E1 |
| Machine profile and limits | Maximum stitch (12.1 mm ternary, 12.7 mm binary), maximum jump, minimum stitch, trim encoding, needle count | B | **done**: profiles `generic` and `commercial15`; long stitches and jumps split, travel that would read as a trim gets one (measured from where the lock stitches leave the needle; none at the start of a colour, which the colour change has cut), warnings for needle count and field size; maximum and minimum stitch and jumps per trim are export options, needles, field and maximum jump are not | E1 |
| Running stitches | Run, triple (bean), backstitch, stem stitch; stitch length, minimum length, chord gap on curves | B | **partial**: run (stitch length, corners kept, path end hit exactly) and manual along stroked paths; no triple, backstitch or stem stitch | E3 |
| Satin column | Two rails with direction lines; spacing fixed or by width; varying width | B | variable-width stroke outlines (`effects::stroke::width`) give rails for strokes; no rail pairing for filled shapes | E3 |
| Tatami fill | Rows at an angle with staggered penetrations, holes respected, travel hidden under the fill | B | Scribble effect makes hole-aware zig-zag rows (`effects::stylize::scribble`); booleans and regions (`pathops`) | E3 |
| Zigzag and E stitch | Open column stitches | B | nothing | E3 |
| Manual stitch | Each node is one needle point | B | **partial**: export mode `manual` makes each anchor a needle point for the whole document; not yet a per-object stitch type | E3 |
| Underlay | Centre run, edge run, zigzag, double zigzag, tatami; two layers; own settings for lettering | B | inset paths (`pathops::offset_path`) | E3 |
| Pull and push compensation | Widen across the stitch direction, in mm or percent, per side | B | offsets | E3 |
| Auto split, auto jump, stitch shortening | Split satin stitches over a maximum (about 7 mm); shorten on the inside of sharp turns | B | nothing | E3 |
| Lock stitches | Tie in and tie off by rule: always, after a trim or colour change, or by connector length | B | **partial**: one fixed pattern (three 0.6 mm stitches) at the start and end of every run, on or off; no rules or other patterns; length untested on a machine | E3 |
| Trims and connectors | Trim by connector length; connectors as jumps or travel runs | B | nothing | E3 |
| Small-stitch filter | Drop stitches below a minimum on output | B | **done**: stitches shorter than `minStitch` are dropped when a plan is prepared | E1 |
| Fabric presets | One choice sets density, compensation and underlay for the design, with per-object override | B | graphic styles and presets exist as a pattern | E4 |
| Scale with recalculation | Resizing regenerates stitches from outlines, so density is kept | B | objects are vectors; follows from E4 | E4 |
| Change density or stitch count | Scale spacing and length by percent or to a target count | M | nothing | E4 |

### Digitizing and editing

| Feature | Commercial behaviour | Tier | VectorCraft today | Gap |
|---|---|---|---|---|
| Shape and outline input | Rectangle, ellipse, pen, freehand | B | **have**: full drawing toolset | — |
| Stitch properties on an object | Each object carries stitch type and parameters; stitches regenerate on edit | B | appearance, live effects and per-object data exist; no stitch properties | E4 |
| Stitch angle and direction lines | Add, move and remove angle lines on a turning fill | M | nothing | E4 |
| Entry and exit points | Per object, movable | M | nothing | E8 |
| Reshape, knife, weld, remove overlaps, fill holes | Node edit; cut; unite; cut away hidden understitching | M | **have**: node editing, knife, Pathfinder, offset; stitch-aware overlap removal is missing | E8 |
| Outlines and offsets | Border objects from a shape | M | **have**: Offset Path, Outline Stroke | — |
| Sequence list | Reorder by object or colour | B | Layers panel orders objects; no colour-block view | E8 |
| Closest join, auto start and end | Join neighbours at nearest points; set design start and end | B | nothing | E8 |
| Auto-routing and branching | One entry and exit across many objects, hidden travel, fewer trims | P | nothing | E8 |
| Travel through the design | Step by stitch, object, colour or function | B | nothing | E5 |
| Mirror, array, wreath, kaleidoscope | Repeats with resequencing by colour | M | **have**: Repeat (radial, grid, mirror), Transform Each; resequencing is missing | E8 |
| Stitch list editing | Select, move, insert and delete single stitches and functions | M | nothing | E14 |
| Auto-digitize artwork | Click a shape or convert a whole image into stitch objects | M | **have**: Image Trace (colour reduction, regions, centre lines); choosing a stitch type per shape is missing | E10 |

### Lettering

| Feature | Commercial behaviour | Tier | VectorCraft today | Gap |
|---|---|---|---|---|
| Pre-digitized fonts | 130–250 hand-digitized fonts, each with a recommended size range | B | none; fonts live in `storytold/craft-fonts`, and stitch fonts would need original digitizing | E9 |
| TrueType and OpenType conversion | Any font becomes satin or fill automatically | M | **have**: shaping and glyph outlines (`text`); splitting glyphs into satin columns is missing (the only skeleton code is raster and private to `trace`) | E9 |
| Baselines | Line, arc, circle, any shape | B | **have**: type on a path, area type | — |
| Kerning and spacing | Automatic and manual | B | **have** | — |
| Lettering envelopes | Preset and custom envelopes | M | **have**: envelope distort | — |
| Letter joins | Closest join or bottom join; trims per letter or word | B | nothing | E9 |
| Small lettering rules | Run fonts under 5 mm, centre-run underlay at 6–10 mm, minimum column about 1 mm | B | nothing | E9 |
| Monograms | Templates with ornaments and borders | B | nothing | E9 |
| Team names (name drop) | One layout, many names, fitted to a width | P | variables and data-driven graphics exist to build on | E9 |

### Colour, preview and production

| Feature | Commercial behaviour | Tier | VectorCraft today | Gap |
|---|---|---|---|---|
| Thread charts and matching | Per-brand charts with codes; match a palette to a chart | B | swatch libraries, palette quantization to a supplied list (`trace`); no thread data | E6 |
| Colourways | Several colour schemes per design | M | Recolor Artwork exists; no colourway record | E6 |
| Needle assignment | Map colours to needle numbers for the machine | B | nothing | E6 |
| Realistic preview | Stitches drawn as lit thread on a fabric background | B | **partial** (measured by tests): `stitch.preview` draws the export's plan as shaded thread capsules with a soft shadow on a flat fabric colour, as a PNG; no panel in the app, no fabric textures | E5 |
| Stitch simulator | Animated stitch-out, with speed and range | B | nothing | E5 |
| Needle points, connectors, function markers | View toggles | B | **partial** (measured by tests): `stitch.preview` options `showNeedles`, `showJumps` (dashed connectors) and `showFunctions` (trim, stop and colour change rings); no toggles in the app UI | E5 |
| Density map | Colour-coded overlap view | B | **partial** (measured by tests): `stitch.preview {view: density}`, 1 mm cells by thread coverage on a fixed ramp, red at 2 (too dense), with `maxCoverage` and `denseCells`; no legend, no view in the app | E5 |
| Hoops and frames | Library, custom hoops, automatic choice, fit check, centring | B | artboards; no hoop model | E7 |
| Production worksheet | Design picture, size, stitch count, stop table with thread codes and needles, trims, thread and bobbin length, run time, barcode | B | **have**: PDF export, print; no worksheet layout or figures | E7 |
| Estimates | Thread per colour, bobbin, run time at a given speed | B | nothing | E7 |
| Design checker | Small stitches, long stitches, density, trims, hoop fit | B | nothing | E7 |
| Quoting | Price by stitch count | P | out of scope for now | — |
| Send to machine | USB stick, a watched network folder, vendor networks | B | file export; vendor network protocols are not publicly documented | E7 |
| Design library, batch convert | Browse, search, convert many files | B | `vectorcraft-cli convert`; no library view | E2 |

### Machine file formats

| Format | Use | Tier | VectorCraft today | Gap |
|---|---|---|---|---|
| DST | Tajima and most commercial machines; no colours, trims as jump runs | B | **done**: reader and writer, trims as a run of `jumpsPerTrim` jumps, a trim before the travel back to the start, start at the centre or the first stitch, header extents from stitches and travel (not the trim jumps); not yet loaded on a machine | E2 |
| EXP | Melco and Bernina; stitches only | B | **done**: reader and writer; no colour companion file; not yet loaded on a machine | E2 |
| TBF, U01, DSB | Tajima LAN, Barudan; carry needle or function data | M | nothing | E2 |
| PES/PEC, JEF, VP3 | Brother, Janome, Husqvarna and Pfaff home machines | M | nothing | E2 |
| Reading stitch files | Open any of the above as manual stitches | B | **partial**: DST and EXP open as stroked paths, one group per colour, on a layer marked as a stitch file, so an export with default options keeps that layer's needle points (anchor by anchor, no added locks) and its trims (a run the file trimmed before is marked, and a run reached by a jump alone is not trimmed; no travel trims are added, `travelTrims` forces them on or off; no return to the start is added unless `returnToStart` is given) while art drawn on other layers is run-stitched and locked; fuzzed; consecutive blocks of one colour merge on re-export; the other formats are missing | E2 |
| Recognising stitch files | Turn imported stitches back into editable objects | P | nothing | E14 |

### Photo, fur and decorative work

| Feature | Commercial behaviour | Tier | VectorCraft today | Gap |
|---|---|---|---|---|
| Single-colour photo modes | Rows of varying width or spacing from a greyscale image; open meander that never crosses | M | Vector Halftone samples tone into geometry (`object.vectorHalftone`) | E11 |
| Colour photo stitch | Reduce to 7–16 thread colours and fill with run stitching | M | Image Trace quantization and regions | E11 |
| Fur and feathered edges | Layered jagged satin; random stitch length at one or both edges | P | nothing | E11 |
| Fur direction from the photo | Not offered by any package found; digitizers place directions by hand | — | nothing; no direction-field analysis exists | E11 |
| Image preparation | Colour reduction, outline sharpening before conversion | M | 57 pixel effects, edge-preserving smoothing in `trace`; no subject cut-out | E12 |
| Gradient and blend fills | Graded row spacing; layered colour blends | P | gradients as paints; no stitch form | E13 |
| Pattern, motif and curved fills | Programmed penetrations, motif fills and runs, contour, spiral, radial, stipple, maze, guide-curve fills | P | pattern brushes and pattern fills exist as geometry sources | E13 |
| Cross stitch | Fill type and grid conversion | P | Image Trace mosaic | E13 |
| Appliqué | Placement, tack-down and cover stitches; cutter outline | M | offsets, DXF export for cutters | E15 |
| 3D foam, caps, multi-hooping | Foam satin settings; cap sequencing presets; splitting over several hoopings | P | nothing | E15 |
| Sequins, chenille, beads, rhinestones, borer, schiffli | Attachment devices with machine-specific codes | P | out of scope until a machine with the device is available | E16 |

## Gaps, in build order

Hours are estimated. Each gap would be its own specification and plan. Each gap has a Linear issue
in the StitchCraft project (see "Where the work is tracked" above); claim work there, not here.

| # | Gap | Contents | Depends on | Hours |
|---|---|---|---|---|
| E1 | Stitch plan core | **Done** (2026-11-11, measured by tests): stitch plan model, machine profiles, physical limits, small-stitch filter, plan statistics (`stitch.info`), in `vectorcraft-stitch` at L2 | — | 0 (was 25–40) |
| E2 | Machine files | DST and EXP **done** (2026-11-11, measured by tests; not yet stitched on a machine): `vectorcraft-embroidery` at L3, engine adapters, format table rows, property and fuzz tests, round-trip checks, `vectorcraft-cli convert`. Remaining: TBF and U01, then PES, JEF and VP3, and a batch convert view | E1 | 30–55 (was 45–80) |
| E3 | Basic stitch types | Run family, satin column, tatami, zigzag, E stitch, manual; underlay; compensation; split and shortening; lock stitches; trims | E1 | 70–110 |
| E4 | Stitch objects | Stitch type and parameters stored on document objects, regeneration on edit, stitch angle lines, fabric presets, a parameters panel | E3 | 40–65 |
| E5 | Preview and simulator | First pass **done** (2026-11-11, measured by tests): `stitch.preview` in `vectorcraft-stitch` (realistic thread, needle point, jump and function overlays, density map, as PNG). Remaining: simulator, preview panel and view toggles in the app, travel controls, fabric textures | E1 | 20–40 (was 30–50) |
| E6 | Threads | User-importable thread charts, a small original default set, matching, colourways, needle assignment | E1 | 15–25 |
| E7 | Production | Hoop library and fit, worksheet as PDF, thread, bobbin and run-time estimates, design checker, export to a folder | E2, E6 | 30–50 |
| E8 | Sequencing | Entry and exit points, closest join, start and end, colour-block list, overlap removal, auto-routing, branching | E4 | 45–75 |
| E9 | Lettering | Font outlines to satin columns, letter joins, small-letter rules, monograms, team names. Pre-digitized fonts are original work and are sized separately | E3, E8 | 60–110 |
| E10 | Auto-digitize artwork | Pick run, satin or fill per traced shape by its width and form; whole-image conversion | E4, E8 | 30–45 |
| E11 | Photo and fur | Colour-region conversion, single-colour photo modes, direction field from the image, layered direction-following fill with varied stitch length, feathered edges, eye and nose detail pass | E3, E6 | 85–150 |
| E12 | Image model provider | Provider interface at the app layer, one hosted adapter, faithful preparation (cut-out, clean-up), opt-in styles, a likeness check for restyled output, a no-model path. Settles gap G18 in [gaps.md](gaps.md) for this use | — | 25–40 |
| E13 | Decorative fills | Gradient and blend, motif and pattern fills, contour, spiral, radial, stipple, maze, guide-curve fills, cross stitch | E4 | 60–100 |
| E14 | Stitch-file editing | Stitch list, single-stitch edits, recognising imported stitches as objects | E2, E4 | 40–70 |
| E15 | Appliqué, foam, caps, multi-hooping | As listed | E4, E7 | 40–80 |
| E16 | Attachment devices | Sequins, chenille, beads, rhinestones, borer, schiffli | E2 | not estimated |

The stated goal, a pet photo to a machine file, needs E1, E2 (DST and EXP only), the fill part of
E3, E6, E11 and E12. E5 is needed early as well, because the preview is how a result is judged
before a stitch-out.

## Constraints from this repository

- **Layering.** The stitch crate sits at L2 beside `pathops` and `trace`; the format crate at L3
  beside `cad`. Both must build for `wasm32`, so neither may use the network or threads directly.
- **Network.** No document feature makes an HTTP call today. The image model provider lives at the
  app layer, native only at first; the web build gets it when a backend exists.
- **Never crash.** Stitch files, photos and model responses are untrusted input: bounded
  allocations, no panics, fuzz and property tests for every reader.
- **Everything is a command.** Each step of the pipeline is an engine command, so the CLI, MCP and
  the panels share one implementation.
- **Fonts and assets.** No font files in this repository. Stitch fonts, thread data and fabric
  presets must be original or suitably licensed, with a row in `ASSETS.md`.
- **Names.** User-visible strings may not use another vendor's product or company names
  (`cargo xtask brands`). Format labels need neutral wording or a recorded exception.
- **Patents.** Several patents cover image-to-embroidery methods (for example US6370442, US6804573,
  US8798781). Their status needs a check by someone qualified before the photo pipeline ships.

## Open questions

1. Which hosted image model is the first provider, and which styles ship first.
2. Which machine and frame the first stitch-outs run on, and its trim convention (jumps per trim).
3. Whether pre-digitized fonts are in scope, who digitizes them, or whether automatic conversion of
   outline fonts is enough for the first release.
4. Which thread brands the owner uses, and whether those charts may be shipped or must be imported
   by the user.
5. Whether embroidery ships inside VectorCraft or as a sibling app on the same crates.
6. Unverified format and parameter facts (see Sources and confidence) need a pass with the vendor
   manuals and format write-ups reachable.

## Sources

Public documentation only. Pages marked (s) were seen as search snippets, not read in full.

- Ink/Stitch user manual: <https://inkstitch.org/docs/stitch-library/>, <https://inkstitch.org/docs/file-formats/>, <https://inkstitch.org/docs/visualize/>, <https://inkstitch.org/docs/thread-color/>, <https://inkstitch.org/docs/font-tools/>
- PEmbroider README and API notes: <https://github.com/CreativeInquiry/PEmbroider>
- pyembroidery README (format notes, MIT): <https://github.com/EmbroidePy/pyembroidery>
- PES and PEC: <https://github.com/frno7/libpes/wiki/PES-format>, <https://github.com/frno7/libpes/wiki/PEC-section>
- DST and JEF (s): <https://edutechwiki.unige.ch/en/Embroidery_format_DST>, <https://community.kde.org/Projects/Liberty/File_Formats/Tajima_Ternary>, <https://edutechwiki.unige.ch/en/Embroidery_format_JEF>
- Wilcom EmbroideryStudio online help (s): <https://docs.wilcom.com/embroiderystudio/28/en/downloads/QuickReference.pdf>, <https://docs.wilcom.com/embroiderystudio/26/en/OnlineHelp/Release/prodiff_table/Product_Comparison.htm>, <https://docs.wilcom.com/embroiderystudio/26/en/OnlineHelp/Automatic/automatic_photos/Auto-digitize_with_Color_PhotoStitch.htm>, <https://docs.wilcom.com/embroiderystudio/26/en/OnlineHelp/Setup/machines/Color-change_functions.htm>, <https://docs.wilcom.com/embroiderystudio/26/en/OnlineHelp/Production/reports/Stop_sequence_options.htm>
- Hatch (s): <https://hatch.embroideryhelp.net/v3/en/OnlineHelp/New_Release/prodiff_table/Feature_Comparison_Table.htm>, <https://hatchembroidery.com/resources/blog/digitizing-animals-with-hatch-creative-ways>
- Tajima DG by Pulse (s): <https://www.tajima.com/product/dg/>, <https://embroidery.pulsemicro.com/dg15-levels.html>
- Melco DesignShop (s): <https://melco.com/designshop-12-compare/>, <https://melco-service.com/source1/Digitizing_for_Caps.htm>
- Brother PE-Design (s): <https://support.brother.com/g/s/hf/ped_info/ped11/new_features/en/ped11_newfeatures.html>
- Embird (s): <https://www.embird.net/sfumato-stitch>, <https://www.embird.net/studio/manual/3075par_fill.htm>
- Embrilliance (s): <https://embrilliance.com/product/density-repair-kit>
- mySewnet (s): <https://softwarehelp.mysewnet.com/MSW/140/Digitizing/Topics/Feathered_Satin_Options.htm>
- Fur and animal practice (s): <https://impressionsmagazine.com/process-technique/digitizing-realistic-designs-part-3-animals/16283/>
- Liu, Piovarči, Hafner, Charrondière, Bickel, "Directionality-Aware Design of Embroidery Patterns", Computer Graphics Forum 42(2), 2023: <https://doi.org/10.1111/cgf.14770>

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-11 | minor | E5 first pass, measured by tests: `stitch.preview` renders the plan an export would write as lit thread on fabric or as a density map, with optional jump, needle point and function marks, as a PNG (design: [the preview spec](superpowers/specs/2026-11-11-embroidery-preview-design.md)). Rows updated: realistic preview, needle points and function markers, density map (each partial). E5 reduced to 20–40 h (estimated) |
| 2026-11-11 | minor | An opened stitch file re-exports with the trims it had and no more: a run the file trimmed before is marked on its path, runs reached by a jump alone are no longer trimmed, and travel trims are off on that layer by default (`travelTrims` on `document.export` and `stitch.info` forces them on or off). A trim before a colour change is not kept (`trimBeforeColorChange` writes one). Row updated: reading stitch files |
| 2026-11-11 | minor | Execution tracking moved to Linear (team StitchCraft, `STI`; project `P-STI-31`): one issue per gap with blocking relations from the "Depends on" column, five owner-decision issues, the calibration stitch-out and its follow-ups, and the re-export travel-trim observation. This document keeps behaviour, status and evidence; scores and estimates unchanged |
| 2026-11-11 | minor | The opened-file defaults apply per layer: art added beside an opened stitch file is run-stitched as in a fresh document, `stitch.info` and a `selectedOnly` export agree, and a warning says when the file's runs get lock stitches too. Row updated: reading stitch files |
| 2026-11-11 | minor | Review fixes, measured by tests: DST and EXP end with a trim before the travel back to the start; travel trims are decided after lock stitches; template and non-printing layers and clipping paths are not sewn (clipped art is sewn in full, with a warning); a document opened from a stitch file re-exports its needle points by default; `stitch.info` reports `records`; DST header extents leave out trim jumps. Rows updated: machine profile, DST, reading stitch files |
| 2026-11-11 | minor | E1 done and the DST and EXP part of E2, measured by automated tests and a manual cross-check with an independent reader; no file stitched on a machine yet. Rows updated: stitch plan model, machine profile, running and manual stitches, small-stitch filter, lock stitches, DST, EXP, reading stitch files. E2 reduced to 30–55 h (estimated) |
| 2026-11-11 | new | First gap analysis from a survey of public documentation. Nothing is implemented; all hours are estimated; commercial feature details are from search snippets and are unverified where marked |
