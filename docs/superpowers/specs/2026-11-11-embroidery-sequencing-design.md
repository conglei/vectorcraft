# Embroidery sequencing, first pass: design

> **Status:** draft for owner review · **Date:** 2026-11-11 · **Covers:** the first part of gap E8
> in [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-19) · **Builds on:**
> [the stitch core](2026-11-11-embroidery-stitch-core-design.md) and
> [the stitch objects](2026-11-11-embroidery-stitch-objects-design.md)

## Goal

Say in which order objects are sewn and where each is entered and left, so a design has fewer
trims, less travel and fewer thread changes. Until now objects sewed in document (layer) order,
bottom to top, each from the start of its path.

In scope: per-object entry and exit points, an explicit sewing order, a design start and end,
closest join, sort by colour, the commands for them and a thin Sequence panel. Deferred: a canvas
tool to drag entry and exit points (and its cursor), stitch-aware overlap removal, auto-routing
and branching, resequencing after Repeat.

## Sources consulted

| What | URL | Licence | Taken |
|---|---|---|---|
| Ink/Stitch user documentation, "Stitch Path Optimization" | <https://inkstitch.org/tutorials/routing/> | user docs of a GPL project (docs only; no code read) | Behaviour only: fills and satins start near the previous element and end near the next; fewer trims matter more than a little more travel (trims make a mess of the back and slow the machine); reversing a path changes its direction |
| The reference embroidery suite's help, "Auto-sequence entire designs" and "Sequencing embroidery objects" | <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Modifying/combine/combine-16.htm>, <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Modifying/combine/combine-12.htm> | public help pages | Behaviour only: colour sorting aims at fewer colour changes and trims, and is one undo step; overlaps are preserved (deferred here, see below) |
| pyembroidery | <https://github.com/EmbroidePy/pyembroidery> | MIT | Searched for an ordering pass; it reads and writes files and has none, so nothing was taken |
| Nearest-neighbour ordering and dynamic programming over a fixed order | textbook material (travelling-salesman heuristics; Viterbi-style shortest path over a layered graph) | — | The algorithms below are written from these general ideas |

A search also turned up US patent 12,241,193 B2 ("displaying and editing the sequence of same
colored parts of an embroidery design", with entry and exit points recalculated after a manual
reorder). It was not read beyond its title and abstract. Owner: decide whether it needs a look
before E8 goes further; nothing here searches entry points on a 0.1 mm grid or draws a sequence
view of same-colour parts.

No code, constant or string was copied, so nothing is added to `NOTICE`.

## Data model

**Entry and exit: a fraction along the outline.** `StitchProps` gains `entry` and `exit`
(file keys `entry`, `exit`), each a number 0–1: how far along the object's outline the point is,
by arc length, with a compound path's subpaths one after another. Chosen over a document-space
point because a fraction moves with the object: moving, scaling or reshaping an object keeps its
entry at the same place on its outline, where a stored point would be left behind (stitches are
made again from the outline at every export, so nothing else may go stale). Commands accept a
document point too and store its projection onto the outline (`outline_fraction`). Values out of
range read as unset, like the other properties; they are never inherited from a group.

`joinEntry` and `joinExit` are the same, chosen by Closest Join. Kept apart from the user's so
that the next closest join may choose again while the user's points stay fixed ("pinned").
`stitch.clear` and `stitch.entryExit.set {clear}` remove both kinds.

**Sequence and start/end: `Document::stitch`, a `StitchDesign`** (file key `stitch`, omitted when
empty, read leniently so a value written differently reads as empty):

| Field | Meaning |
|---|---|
| `order: [id…]` | objects sewn first, in this order; every other sewn object follows in document order. Unknown or deleted ids are skipped; a repeated id counts once |
| `start: {x, y}` | design start, document coordinates: the first object is entered nearest it and a stitch file takes it as its origin (the export's `start` option is the fallback when it is unset) |
| `end: {x, y}` | design end: the last object is left nearest it |

A partial order (rather than a full permutation) keeps old files and new objects working: a file
without `stitch` sews exactly as before, and an object drawn after the order was set sews last,
which is where it would be in document order. No format version bump.

**Room for later.** The sequence unit is an object (a path or compound path). Auto-routing and
branching will add a *route*: a list of objects joined by hidden travel runs into one entry and
one exit. It fits as another kind of entry in `order` (`{route: [ids…]}`; today's reader drops a
value it doesn't know as unset, so it would lose the order but open the file) and as another
`Way` cost in the algorithms below, whose inner travel can then be sewn under other objects
instead of jumped.

## Algorithms (`crates/stitch`, pure, std only)

The walk in `from_document` now collects each object's runs (a `Unit`) instead of sewing them at
once; the units are then sewn in the design's order, each turned one of its **ways**:

- forwards (as before);
- backwards: each stitch type's runs in reverse, each run reversed, the types in their order (a
  fill stays under its outline);
- a single closed run from another of its stitch points: up to 48 sampled evenly, plus the one
  nearest each of the object's entry and exit points.

An opened stitch file's art has one way (forwards), so its needle points and trims are kept.

`sequence::Item` abstracts an object for the algorithms: the thread it starts and ends in and
its ways, each with an entry, an exit and the trims and travel between its own runs. A **cost**
is (trims, travel, thread changes), counted as `from_document` sews: a join within one thread
farther than `minStitch` is a trim, travel is every distance moved without sewing, from the
design start when set to the design end when set.

- **Which way** (`current_ways`): pinned by the user's points (the way nearest them), else the
  closest-join choice, else forwards; with a design start or end, the first and last objects
  that are not chosen are turned to them.
- **Closest join** (`closest_join`): for the objects in their order, the cheapest ways *exactly*
  by dynamic programming over (object, way): O(n·k²), k ≤ 50. Run twice, ranking trims before
  travel and travel before trims, and the first result no worse than the current ways on both
  trims and travel is taken; otherwise nothing changes. So closest join never adds a trim or
  travel (property-tested on random items and on random documents through `from_document`).
  Greedy nearest neighbour was rejected: it can pick a near entry whose exit is far from the next
  object, ending worse than before.
- **Sort by colour** (`colour_sort`): group by the thread each object starts in, groups in order of
  first appearance, objects in each group in their order (stable). Taken only when it lowers the
  thread changes, so it never raises them; for single-colour objects it reaches the minimum,
  colours − 1. Overlaps between colours are not checked yet (a later object of an earlier colour
  may now sew under a covering object); overlap-aware sorting is deferred with overlap removal.
- **Order** (`resolve_order`): the listed objects first, each once, then the rest in document
  order. Every sewn object is sewn exactly once in any order (property-tested).

## Commands (`crates/engine/src/cmd/stitch/sequence.rs`)

Each change is one undo step through `Session::edit`.

| Command | Does |
|---|---|
| `stitch.sequence.list {options?}` | the order with each object's id, name, colours, stitch types, stitches, entry and exit (document points), its own and closest-join points, `pinned`/`opened`; the colour blocks it makes; trims, travel and colour changes of the order (the model's numbers) |
| `stitch.sequence.move {id, to}` | move one object to index `to` |
| `stitch.sequence.set {ids}` | sew these first, in this order (`[]`: document order again) |
| `stitch.sequence.sortByColor` | sort by colour (above) |
| `stitch.entryExit.set {ids?\|id?, entry?, exit?, clear?}` | the user's points: a fraction 0–1, a point `{x, y}` (projected onto the outline) or `null` to unset; `clear` removes both and the closest-join ones |
| `stitch.join.closest {options?}` | closest join; stores `joinEntry`/`joinExit` → before and after costs |
| `stitch.designStartEnd.set {start?, end?}` | `{x, y}` or `null` |

`stitch.info` gains `travelMm`. Exports, `stitch.info`, `stitch.preview` and the Stitch Preview
panel all go through `from_document`, so they all use the order and the points.

## Sequence panel

`Window › Stitch Sequence` and `Embroidery › Stitch Sequence` (`ui.stitchSequence`): the order as
a list (colour swatch, object name, stitch types, stitches), Move Up and Move Down, Sort by Colour
and Closest Join, the colour changes and trims of the order. A row click selects the object; the
selected objects are highlighted. Every action is an engine command.
