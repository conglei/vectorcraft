# Embroidery document setup: design

> **Status:** implemented (STI-34) · **Date:** 2026-11-13 · **Covers:** the hoop drawn on the canvas, the out-of-hoop check, a new design started from a hoop, the fabric colour, and the Hoop row of Properties. Item 5 of [the workspace design](2026-11-12-embroidery-workspace-design.md); the hoop model and library are in [the production design](2026-11-12-embroidery-production-design.md); the features are tracked in [embroidery-parity.md](../../embroidery-parity.md)

## Goal

The owner typed lettering in StitchCraft and asked for a page setting that chooses the hoop's
size and shape, so the design can be previewed in it. The hoop already exists as a model
(`StitchDesign::hoop`), a library of generic sizes, fit and centring commands and a dropdown in
Stitch Info, but nothing draws it and no document starts from one.

## What other embroidery software does

From the public documentation named in the workspace design's sources; nothing was run. A new
design starts by choosing a hoop, and the design window shows the hoop's outline and the fabric
colour behind the stitches (mySewnet, Embird, Bernina). Designs over the hoop's edge are flagged
before export. Described here in our own words; no text, icon or artwork is copied.

## Decisions

### 1. Hoop on the canvas

When the design has a hoop and View › Hoop is on (`view.hoop {on?}`, kept in the preferences with
the other view toggles, on by default), the canvas draws it in every view (Preview, Outline,
Stitches):

- the hoop's inner edge in its shape: a rectangle with corners rounded 6 mm (less on a small hoop),
  or the ellipse inside width × height for a round hoop;
- a band 7 mm wide outside the edge, for the frame;
- the sewing field, 10 mm (the checker's default margin) inside the edge, as a dashed line;
- the canvas outside the field dimmed;
- the hoop's name over its top-left corner.

It is centred where `stitch.hoop.center` centres the design: the first artboard's centre. Colours
are the `hoop`, `hoop_field` and `hoop_outside` theme tokens. The drawing turns with the view. Per
frame it builds three outlines (96 or 36 points) and two meshes, nothing else. It is not drawn in
Presentation Mode. The menu items are View › Hoop and Embroidery › Hoop on the Canvas.

### 2. Out-of-hoop check

`stitch.check` gains the rule `outsideHoop`, for a design with a chosen hoop: the stitches'
bounds, where they are, against the hoop centred on the first artboard. Over the hoop's edge is an
error (the needle would strike the frame); inside the edge but within `marginMm` of it is a
warning. When the design would fit centred, the message names Center Design in Hoop as the remedy.
Art that sews nothing does not count, because the rule reads the stitch plan, not the art. Without
a chosen hoop the existing `offCenter` note about the fitted hoop stays as it was.

The thread bar under the canvas reads the same placement from the plan it already holds (its
bounds were added to the plan summary) and shows "Outside the hoop" in the error colour or "Near
the hoop's edge" in the warning colour, with the explanation on hover. Stitch Info's Design Check
lists the finding with the others.

### 3. A new design from a hoop

`file.new` learns `hoop` and `fabric`. The hoop is a library id, `{id}` or
`{widthMm, heightMm, shape?, name?}`; without `width` and `height` the artboard is the hoop's size.
The document is born with them, so there is no undo step to merge: this is the cleaner route than
running `stitch.hoop.set` after `file.new`, which would leave a "Set Hoop" step in a new
document's history.

New Document shows a Hoop section when the app's work is embroidery (StitchCraft: the identity's
`stitch_view`, which the UI knows as `VectorcraftApp::embroidery`), never in VectorCraft: the
hoop dropdown (None, the library, Custom with width, height and shape) and the fabric colour.
Choosing a hoop sets the width and height to its size. A StitchCraft New Document opens on the
130 × 180 mm rectangle. The view fits the new artboard, which is the hoop.

Document Setup gets an Embroidery tab in StitchCraft, or for any design that has a hoop: the same
section and "Resize the artboard to the hoop". `document.setup` takes them as
`embroidery: {hoop?, fabric?, resizeArtboard?}` in its one undo step, and reports `embroidery`.

### 4. Fabric colour

`StitchDesign::fabric`, `#rrggbb` in the file, absent for the default (#ece8de), read leniently
(another value reads as the default; no format change). Set with `stitch.setup {hoop?, fabric?,
resizeArtboard?}`, which sets the hoop and the fabric together in one undo step, reports them with
`{}`, and takes its own reply back. `stitch.hoop.get` reports the fabric too. The fabric is the
background of the canvas's stitch view and its underlay wash, of the Stitch Preview panel's
picture, of `stitch.preview` when no `fabric` is given, and of the worksheet's design picture.

### 5. Properties

The Document section of Properties shows a Hoop row (the dropdown Stitch Info uses, shared) and the
fabric colour swatch, when the app's work is embroidery or the design has a hoop. A colour drag
commits once, when the pointer is released.

## Not done here

- The hoop's margin is the checker's default (10 mm); it is not stored per design.
- New Document does not remember the last hoop used; it opens on 130 × 180 mm.
- Hooping several times (multi-hoop designs) and hoop-specific machine fields.
