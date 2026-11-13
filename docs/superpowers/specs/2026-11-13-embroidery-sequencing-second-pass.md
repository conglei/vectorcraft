# Embroidery sequencing, second pass: design

> **Status:** draft for owner review · **Date:** 2026-11-13 · **Covers:** the rest of gap E8 in
> [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-35, fixing STI-32) · **Builds on:**
> [the first pass](2026-11-11-embroidery-sequencing-design.md)

## Goal

What the first pass deferred: drag entry and exit points on the canvas, cut the hidden stitching
under a later fill, join objects of one thread by hidden travel instead of a trim, and make the
Stitch Sequence panel count what the export sews (STI-32).

In scope: one accounting for the list, the Entry/Exit tool and its cursor, `stitch.overlap.remove`
and `stitch.overlap.clear`, `stitch.route`, and the shared travel code. Deferred: resequencing
Repeat (step and repeat) copies by colour, branching inside one object, and travel that bends
round obstacles (it is a straight line here, or a trim).

## Sources consulted

Documentation only, read for behaviour; no code, constant or string was copied, so nothing is
added to `NOTICE`.

| What | URL | Taken |
|---|---|---|
| The reference embroidery suite's help, "Adjust entry/exit points" | <https://docs.wilcom.com/embroiderystudio/27/en/OnlineHelp/Quality/connectors/connectors-17.htm> | Entry and exit markers are shown on the selected object and dragged; dragging them close to a neighbour's tightens the join |
| The same help, "Removing underlying stitching" | <https://docs.wilcom.com/embroiderystudio/26/en/OnlineHelp/Quality/quality/Removing_underlying_stitching.htm> | The later object is the cutter; an overlap amount keeps a band under its edge so no gap shows |
| A sewing-software help, "Hide travel runs" and "Embroidery connectors" | <https://softwarehelp.bernina.com/en/MainHelp/PART_Decorative/decorative/decorative-9.htm>, <https://softwarehelp.bernina.com/en/MainHelp/PART_Digitizing/connectors/connectors-1.htm> | Travel runs are kept where later stitching hides them; elsewhere a jump and a trim |

A blog round-up of overlap settings gave 0.5–1 mm as the usual overlap; the default here is
0.7 mm.

## STI-32: one accounting

The list estimated a trim between every two objects of a colour whose ends were apart, and
counted needle points before lock stitches. The export joins an opened file's and a photo
design's runs only where the file trimmed, joins letters by a stitch or a jump, and adds lock
stitches and the machine's trims. So the panel said 53 trims where `stitch.info` said 16, and 637
stitches where the player said 652.

Now `from_document` tags every step with the object it sews (`Planned::owners`, `Planned::tags`;
the jump and trim leading to an object are its own), `prepare_tagged` carries the tags through
preparing the plan (a lock stitch, a trim before travel and a piece of a split stitch take the tag
of the step they are added for), and `stats_by_tag` counts them as `stats` counts the plan.
`stitch.sequence.list` takes each object's `stitches`, `trims`, `jumps` and `travelMm` and the
totals from the plan `stitch.info` counts with the same options. The sequencer's own cost
(`sequence::cost`) still steers Closest Join; it is no longer shown.

## Entry/Exit tool

`stitchEntryExit` (Embroidery › Entry and Exit Tool; the All Tools drawer). The selected paths and
compound paths show their entry (a green ring) and exit (a red ring) where the sewing order puts
them. Dragging one moves it along the outline: Begin, Preview `stitch.entryExit.set {ids:[id],
entry|exit: fraction}`, Commit, one undo step. Within 8 screen pixels of the previous object's
exit (dragging an entry) or the next object's entry (dragging an exit) it snaps to the outline
point nearest that point. Shift sets entry and exit to the same place. Escape cancels. The cursor
is `Cursor::StitchEntryExit` (a crosshair stand-in until drawn).

## Overlap removal

`StitchProps.overlap` (file key `overlap`, mm, 0–5, the object's own, never inherited): sew this
fill without what later fills cover. At every plan, a fill with it set is sewn over its region
minus the union of the regions of the fills sewn after it, each inset by the allowance (Pathfinder's
offset and booleans), so the later fill still overlaps the cut edge by the allowance and no fabric
shows. Moving either object re-cuts it; the stored value is only the allowance.

`stitch.overlap.remove {ids?, allowanceMm?: 0.7}` sets it on each listed fill (default: every
sewn fill) that a later fill overlaps, one undo step, and reports stitches before and after.
`stitch.overlap.clear {ids?}` removes it. The Stitch Properties panel shows "Overlaps removed
(0.7 mm)" with a Clear button.

`stitch` gains a dependency on `pathops` (both L2; `xtask layers` orders them).

## Auto-routing within a colour

`StitchProps.route` (file key `route`, mm, 0–20, own): the most travel by which this object may
be entered from the one before it without a trim. When the two are one thread and the needle is
further than the join distance, the straight way from the exit to the entry is stitched as
running stitch (stitches no longer than the run length) when every point of it lies inside a fill
of the same thread sewn from this object on (this object's own fill included) or within 0.3 mm of
its edge (a shared edge); otherwise the trim and jump stay. `stitch.route {ids?, maxTravelMm?: 3
(0: clear)}` sets it, one undo step, and reports trims before and after.

The travel code is one module, `vectorcraft_stitch::travel` (`winding`, `inside`, `hidden`,
`stitched`): photo designs (`travelMm`) use it in place of their own copy, and lettering's short
joins are the same straight stitch.

## Tests

- A photo document and a lettering document: `stitch.sequence.list` totals and the sum over its
  objects equal `stitch.info` (stitches, trims, travel, colour changes).
- The tool: hover over a handle shows its cursor; a drag previews and commits
  `stitch.entryExit.set`; snapping; Shift sets both. A headless canvas frame draws the handles.
- Two overlapping 20 mm squares: the earlier's stitches drop; none of its stitches lie further
  than the allowance inside the later square; outside the overlap they are unchanged.
- Routing: two same-colour squares sewn one into the other lose their trim; a gap over the
  limit, or not under a later fill, keeps it.
