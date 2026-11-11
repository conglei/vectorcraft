# Embroidery stitch objects, first pass: design

> **Status:** draft for owner review · **Date:** 2026-11-11 · **Covers:** the first part of gap E4
> in [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-15) · **Builds on:**
> [the stitch core](2026-11-11-embroidery-stitch-core-design.md) and
> [the stitch types](2026-11-11-embroidery-stitch-types-design.md)

## Goal

Each object says how it is sewn. Until now the export options `fill` and `stroke` decided the
stitch type for the whole document. This pass stores a stitch type and its parameters on paths,
compound paths and groups, makes the export read them first, and adds commands and a panel to set
them.

## Sources consulted

| What | URL | Licence | Taken |
|---|---|---|---|
| Roidery README and ROIDERY.md | <https://github.com/lork-fork/Roidery> | MIT OR Apache-2.0 | Confirmation only: settings are stored per object; with none, a fill is tatami and a stroke is running stitch; a design-wide preset can apply to objects. Roidery stores the settings as JSON in object attributes; this design uses a typed struct instead (see below). Its source was not read |
| OpenStitch README | <https://github.com/cristianonescu/OpenStitch> | MIT OR Apache-2.0 | Confirmation only: a per-object attributes pane with type, direction, density, stitch length, pull compensation and underlay. Its source was not read |

No code, constant or string was copied, so nothing is added to `NOTICE`.

## Data model

`vectorcraft_doc::StitchProps`, on the node's extras (`NodeExtras::stitch`, `Option<Box<…>>`), like
the other per-object structured data (Gap Options, slice options, perspective). Every field is
optional; empty properties are removed, so an object without any costs nothing.

| Field (file key) | Values | Used by |
|---|---|---|
| `type` | `auto` (default), `none`, `run`, `satin`, `tatami`, `manual` | all |
| `fillAngle` | −360–360 degrees, counter-clockwise | tatami |
| `fillSpacing` | 0.2–5 mm between rows | tatami |
| `fillStitchLength` | 1–8 mm along a row | tatami |
| `stagger` | 0–1, a fraction of the stitch length | tatami |
| `satinSpacing` | 0.2–5 mm, peak to peak | satin |
| `satinWidth` | 0.5–20 mm, rail to rail (unset: the stroke weight) | satin |
| `runLength` | 0.5–12 mm | run |
| `underlay` | bool | stored only, not sewn yet (E3) |
| `compensation` | 0–2 mm | stored only, not used yet (E3) |

The ranges are those of the export options, so an object can say anything the options can.

**Stitch types.** `auto` is today's rule: the fill as tatami, then the stroke as running stitch
(or satin, or anchor by anchor), as the export options say. Any other type sews the object once:
`tatami` fills the region the outline encloses in the fill colour (the stroke colour when it has no
fill); `run`, `satin` and `manual` sew the outline in the stroke colour (the fill colour when it has
no stroke). `none` leaves it out. A satin column along an object without a stroke is 3 mm wide
unless a width is given.

## Defaults and inheritance

Each field is found in this order:

1. the object's own value;
2. the nearest group, layer or compound path holding it that sets that field (field by field, so
   a group can set the type and a path inside only its row spacing);
3. the document default: the export's stitch options (`fill`, `stroke`, `stitchLength`, `mode`;
   the Stitch File Options dialog's Filled Paths and Stroked Paths);
4. the built-in default of those options.

A compound path's members are sewn as the compound path, so its properties are the compound
path's; a command given a member changes its compound path. Values read from a file outside their
range count as unset. On the layer an opened stitch file made, stitch properties are not used: its
art keeps its needle points, as before.

## Regeneration

Stitches are never stored. Each export, Stitch Info and Stitch Preview builds the plan from the
current outlines with the current properties (`vectorcraft_stitch::from_document`), so moving,
scaling or reshaping an object sews it again at the same density: a filled square scaled by 2 sews
about 4 times the stitches with the same row spacing (tested in `vectorcraft-stitch` and through
`object.scale` in the engine). The properties travel with the object through transforms, copy and
paste, and grouping.

## Commands

| Command | Params | Result |
|---|---|---|
| `stitch.setType` | `{type, ids?|id?}` | `{changed: [id…]}`; one undo step |
| `stitch.setParams` | `{params: {fillAngle?…, underlay?}, ids?|id?}`; null unsets a field | `{changed}`; one undo step |
| `stitch.clear` | `{ids?|id?}` | `{changed}`; one undo step |
| `stitch.props` | `{ids?|id?, options?}` (the export's stitch options as defaults) | `{objects: [{id, kind, own, type, sews, params, opened}]}` |

They default to the selection, take paths, compound paths, groups and layers, skip other kinds,
and refuse an empty result, an unknown id, an unknown parameter or a value out of range (nothing
changes). Plug-ins may call all four.

## Panel

Window › Stitch Properties, also in the Embroidery menu. For the selection: a Stitch Type dropdown
("Mixed" when the objects differ), what an export sews ("Sewn as"), the fields of what is sewn
(blank when the objects differ), and Reset to Automatic. It reads `stitch.props`, passing the
Stitch File Options used last as the defaults, and acts through the commands. The Stitch File
Options dialog says that its Filled Paths and Stroked Paths are the document defaults.

## File format

Objects gain an optional `stitch` member. The document version (`FORMAT_VERSION`, 3) is not
raised: that number marks changes whose meaning needs converting on opening (Twist direction,
halftone radii), and an optional member defaulting to automatic needs none, the way the other
optional object members were added. A file without `stitch` opens with every object automatic;
a `stitch` member of another kind, an unknown type or a value of the wrong kind reads as unset, so
a later version's or a damaged file still opens. Tested in `crates/format/tests/prop_format.rs`
(a property round trip in every version written, and the old and unreadable files).

## Tests

- `vectorcraft-doc`: inheritance, ranges, serialization, lenient reading.
- `vectorcraft-format`: property round trip; files without or with unreadable stitch properties.
- `vectorcraft-stitch`: each type, overrides over the options, group inheritance, opened files,
  scaling, and `sewn_as` against the plan.
- `vectorcraft-engine`: the four commands, undo, refusals, groups, compound paths, scaling;
  `command_sweep` and the plug-in review list.
- `vectorcraft-ui-egui`: headless frames of the panel (type, fields, Mixed, no selection, defaults
  from the Stitch File Options, menus).

## Deferred

- Stitch angle and direction lines (a tool to draw them on a turning fill).
- Fabric presets that set density, compensation and underlay for the design.
- Change density or stitch count by a percentage.
- Entry and exit points per object (E8).
- Underlay and pull compensation generators (E3, second pass): the fields are stored now.
- Using stitch properties on an opened stitch file's layer (converting its runs to objects, E14).
