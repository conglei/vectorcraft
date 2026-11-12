# Embroidery workspace: design

> **Status:** items 1 to 4 implemented (STI-31), without the sequence link of item 2 and with the design read-out on the thread bar; item 5 not started · **Date:** 2026-11-12 · **Covers:** how StitchCraft's window is arranged for embroidery: the stitch view on the main canvas, the stitch player bar, the Embroidery workspace, the status readout and the thread bar. The stitch features themselves are in [embroidery-parity.md](../../embroidery-parity.md); the preview renderer is in [the preview design](2026-11-11-embroidery-preview-design.md); the app shell is in [the app shell design](2026-11-11-stitchcraft-app-shell.md)

## Goal

A person who opens StitchCraft sees what the machine will sew, large, in the place they work. Today
they see vector hairlines on a paper page and a small stitch picture at the bottom of a side
column.

## What is wrong today

Measured on a screenshot of StitchCraft's first-start layout (3024 × 1898 pixels, six run-stitch
lines):

- The stitch picture is about 4% of the window. It is the last of four groups in the right column,
  under Properties, Stitch Properties and Stitch Sequence. Its play button, slider and colour-block
  list are below the window's edge.
- The canvas is about 60% of the window and shows no stitches.
- A column of illustration panels (Color Guide, Brushes, Symbols, Pattern Options, Gradient,
  Transparency, Glyphs, Variables and others) stands between the canvas and the embroidery panels.
- The document is measured in points on a letter-size page. There is no hoop, no millimetre grid
  and no fabric colour on the canvas.
- The status bar shows render timings. It does not show the stitch count, the size or the number
  of colours.
- The workspace is Essentials. The first start only docks two panels onto it
  (`first_start_panels` in `apps/stitchcraft/src/main.rs`).

## What other embroidery software does

From public documentation only (see [Sources](#sources)); nothing was run. The points that most
products share:

1. The realistic view is the main design window, toggled in place (Wilcom and Hatch, mySewnet,
   Embird). It is not a side panel.
2. The stitch player plays on the main design window, and pauses at colour changes, trims and
   stops (Wilcom, mySewnet).
3. One list in sewing order, grouped by colour, stays in step with the canvas (Wilcom, Embird,
   PE-Design, Embrilliance, mySewnet).
4. The threads the design uses are shown along an edge, apart from the thread chart.
5. A bottom status bar carries the stitch count and the design's size (Wilcom, Hatch, Embird).
6. The hoop is drawn on the canvas, and the fabric colour is the backdrop (mySewnet, Embird,
   Bernina).

Where they are weak, and where this design differs:

- The player is a mode apart from editing, and Wilcom's slider only sets where playback starts.
  Here the player is a bar that is always under the canvas, with a scrubber over the whole design.
- Hatch's documentation advises leaving the realistic view off while editing. Here the realistic
  view is built off the UI thread and only for the part of the design on screen, so it can stay
  on.
- Size and stitch count are read-outs. No product was found that marks an out-of-hoop or
  too-dense area on the canvas (unverified). That is item 5 below and the density view.

## Decisions

### 1. Stitch view on the main canvas

A third way to draw the document, beside Preview and Outline.

- **State.** `stitches: bool` beside `outline` in the per-document `View`
  (`crates/ui-egui/src/state.rs`). A document opens in Preview in VectorCraft and in the stitch
  view in StitchCraft. Outline and the stitch view exclude each other: turning one on turns the
  other off.
- **Command.** `view.stitches {on?}`, a UI command (`crates/ui-egui/src/menus/ui_commands.rs`), in
  View after Outline and at the top of Embroidery (Stitches on the Canvas), with the shortcut
  Alt+Y (Cmd+Y is Outline, Cmd+Shift+Y and Cmd+Alt+Shift+Y are taken). The Stitch Preview panel
  has an On the canvas box for it. The Embroidery workspace's control bar gets a three-way switch
  (Artwork, Stitches, Outline) with item 3.
- **What is drawn.** The pasteboard as now. Each artboard's paper in the fabric colour (the
  preview's default `#ece8de` until documents carry a fabric colour). The stitch plan as lit
  thread, at the document's own position and at the canvas zoom. The vector art itself is not
  drawn, but selection marks, anchors, handles and tool overlays are, so objects can still be
  picked, moved and reshaped. Hit testing is unchanged: it works on the document.
- **A faint artwork underlay** (the art at low opacity under the thread) is a toggle, off by
  default. It helps when an object has no stitches yet.
- **Toggles.** Thread or Density, Needle points, Jumps and trims: the settings the Stitch Preview
  panel has today (`panels::stitch_preview::Settings`). They become the canvas's settings; the
  panel reads the same ones.
- **Placement.** The plan's points are already in document space: `Pt::from_pt` converts document
  points, and the file origin is applied only by the writers (`stitchfile::written`). The preview
  layout knows the left and top of its picture in millimetres but does not return them.
  `vectorcraft_stitch::preview` gains a way to draw a given rectangle (in mm) at a given scale,
  and `vectorcraft_engine::stitch_preview::BuiltPlan` gains `region(rect_mm, scale, options)`
  beside `frame`. The canvas asks for the visible rectangle at the view's pixels per millimetre.
- **Limits.** The region's size is the canvas's size in pixels, at most the renderer's `MAX_SIDE`
  (8192) a side; a larger canvas gets a coarser picture, scaled. The scale is 0.05
  (`MIN_REGION_SCALE`, below the whole picture's `MIN_SCALE` so a zoomed-out canvas is not asked
  for a huge picture) to `MAX_SCALE`; beyond 50 px/mm the picture is magnified. A rotated view
  (the Rotate View tool) draws the region for the bounding rectangle and turns the texture.
- **No fabric in the picture.** The region is drawn transparent where there is no thread, with
  straight alpha, and the canvas paints the artboards in the fabric colour under it. Stitches
  outside an artboard show on the pasteboard.
- **As built.** The plan, its worker and the playback position stay in the panel's module
  (`panels::stitch_preview::for_canvas`); the canvas keeps its own picture
  (`canvas/stitches.rs`). The underlay is the art's own frame under a wash of the fabric colour.
  A second pane of a document (View › New View split) shows the art, not the stitches.
- **Speed.** The plan is built by the worker the panel already uses, a quarter of a second after
  the last change. The region is drawn by the same kind of job when the view, the toggles or the
  playback position change. While a new picture is on its way, the old one is drawn stretched to
  its new place, so panning and zooming never show a blank canvas. Each stitch outside the
  region is clipped to nothing on its own; skipping whole blocks by their bounds is left for the
  perf pass.
- **No stitches.** A document with nothing to sew shows the fabric, the selection marks and one
  line of text saying why (the plan's error), with the underlay forced on so the art is not lost.
- **Web.** There are no threads on the web: the plan and the region are made at once, as the
  panel does now. Large designs will stutter there; a budget is set in the perf pass.

### 2. Stitch player bar

A bar under the canvas, shown when the stitch view is on (and hidden with it).

- **Controls, left to right.** To start, back one colour block, play or pause, forward one colour
  block, to end. Speed (the panel's five speeds). The scrubber. A read-out: stitch *n* of *N* and
  the active block's colour.
- **The scrubber** spans the rest of the bar. It is drawn as one segment per thread block, in the
  block's colour, as wide as its share of the stitches, with ticks at trims and stops. Dragging
  sets the position; clicking a segment jumps to its start. Left and Right step one stitch, with
  Shift ten, while the bar has focus.
- **Playback happens on the canvas.** The position limits the region render to the first *n*
  stitches (`FrameOptions::stitches`), and the needle position is ringed, as in the panel.
- **Pause at.** A menu with Colour changes, Trims and Stops; each on makes playback stop there.
  Off by default.
- **Follow.** When the needle leaves the visible area during playback, the view pans to it. On by
  default; off in the same menu.
- **Link to the sequence.** Selecting a row in Stitch Sequence moves the position to that object's
  first stitch. Scrubbing highlights the row being sewn. This needs the plan to say which object
  each run came from; if `StitchPlan` does not carry it, that is a first step of this item.
- **State and commands.** The runtime state (position, playing) moves out of the panel's
  thread-local into the app, so the bar, the panel and the canvas share it. `ui.stitchPreview`
  keeps its parameters and gains `pauseAt` and `follow`.
- **As built.** The bar is a bottom panel of the window, over the status and help bars, so it
  runs under the tools and the panels too, not only under the canvas: a longer scrubber, and no
  change to the canvas's own rectangle. Its buttons are drawn in code (triangles and bars). The
  read-out is "stitch / total · colour n/N". Stops tick the full height of the scrubber in the
  warning colour; trims tick short. The runtime state stayed in the panel's module, which the bar
  and the canvas call (`player`, `seek`, `toggle_play`, `step_block`, `nudge`). The panel keeps
  its own play row while the canvas shows the art, and drops it in the stitch view.
- **Not built: the link to the sequence.** `StitchPlan` does not record which object a run came
  from. Adding that changes `crates/stitch/src/from_doc.rs`, which STI-30 is editing, so it waits
  for that to merge.
- **The Stitch Preview panel stays.** It becomes the overview: the whole design, small, with a
  frame showing what the canvas shows. Its own play row is removed; its figures and colour-block
  list stay until the status bar and the thread bar replace them (item 3 and 4), and then go.

### 3. The Embroidery workspace

- **A built-in workspace** named Embroidery in `workspaces::builtins()`. It is StitchCraft's
  default; in VectorCraft it is one more entry in Window › Workspace.
- **Layout.** Tools on the left. On the right one column: Stitch Sequence on top, then a group
  with Stitch Properties and Properties as tabs, then Layers. No icon column. Control bar on, with
  the view switch. Rulers on. The player bar and the thread bar under the canvas.
- **Illustration panels** are not in this workspace. They stay in the Window menu and open
  floating, as any panel the workspace has no place for.
- **First start.** `AppIdentity` gains `default_workspace: &'static str`, which replaces
  `first_start_panels`. StitchCraft names Embroidery. Reset Workspace returns to it.
- **Units.** `AppIdentity` gains `default_unit`. StitchCraft's new documents use millimetres, and
  its rulers and grid follow the document. Existing documents keep their unit.
- **Status bar.** With a plan, the status bar shows: stitches, colours, width × height in mm,
  trims. The figures come from `PlanSummary`. The render and frame-rate timings move behind
  Preferences in StitchCraft. While the plan is rebuilding the figures are dimmed. A warning count
  opens Stitch Info.
- **Tools.** The toolbar keeps every tool. Hiding tools is a later decision for the owner (the app
  shell design, decision 3).

**As built (items 3 and 4).** The workspace is `workspaces::EMBROIDERY` with the layout
`panel_docking::embroidery_layout`; `AppIdentity::default_workspace` and `default_unit` replace
`first_start_panels`, and a first start of StitchCraft switches to Embroidery and sets the General
unit preference to millimetres, so new documents and the rulers use them. Differences from the
text above:

- The design's figures (stitches, colours, width × height in mm, trims) are on the thread bar's
  right, not in the status bar. The status bar's fields have fixed, measured widths, and the
  figures belong with the threads; the status bar is unchanged, timings included.
- The three-way Artwork, Stitches, Outline switch in the control bar is not built. View ›
  Stitches, Alt+Y and the panel's On the canvas box switch the view.
- The warning count that opens Stitch Info is not built.
- The Stitch Preview panel is not in the workspace; it opens from the Window and Embroidery menus.
- A thread swatch moves the player to where its block starts. It does not select the block's
  objects (the plan does not record them, as for the sequence link), and it shows the block's
  number in the sewing order, not a needle number.

### 4. Thread bar

A strip under the player bar: one swatch per thread block in sewing order, with its needle number
when assigned. Hovering names the thread and its stitch count. Clicking selects the block's
objects and moves the player to its start. It reads `PlanSummary::blocks`. Changing a thread from
the bar is later work (E6).

### 5. Hoop on the canvas

The hoop's sewing field drawn on the canvas, the design centred in it, and whatever is outside
marked in red on the canvas and counted in the status bar. This needs the hoop model of STI-16
(E7) and is not designed here beyond naming it. It is a follow-up issue once STI-16 lands.

## Order of work

1. Item 1: region render in `vectorcraft-stitch` and the engine, `view.stitches`, the canvas
   drawing, StitchCraft opening documents in the stitch view.
2. Item 2: shared playback state, the player bar, pause-at and follow, the sequence link.
3. Items 3 and 4: the workspace, `default_workspace` and `default_unit`, the status read-out, the
   thread bar, trimming the panel.

Each is one pull request with its own task id, tests and doc updates. Item 1 alone answers the
complaint that started this: the stitches fill the canvas.

## Tests

- `vectorcraft-stitch`: a region drawn at scale *s* equals the same rectangle cut from the whole
  picture at scale *s*; a region outside the design is fabric; a region larger than the limit is
  an error, not an allocation.
- Engine: `BuiltPlan::region` over the whole design equals `frame` at the same scale.
- UI, headless frames (as the Stitch Preview panel tests): `view.stitches` turns Outline off and
  back; the canvas shows a stitch texture after the quiet moment; a selection mark is still drawn;
  an empty document shows the reason; the player bar's position limits the stitches drawn; pause-at
  stops on a colour change; the Embroidery workspace's layout has the three groups and no icon
  column; StitchCraft's first start uses it; the status bar shows the plan's figures.
- The full `ui-egui` suite runs before each pull request: the menu item lists are tested, and the
  Embroidery menu is also being edited by STI-16 and STI-30.
- `vectorcraft-cli perf` before and after item 1, and a new budget: a region redraw of a
  100,000-stitch design at 1920 × 1200 (the number is set from the first measurement).
- Looked at: `ui.screenshot` of each item in StitchCraft at 1440 × 900 and at the size above.

## Risks

- **Region render cost.** A dense photo-stitch design has hundreds of thousands of stitches. If a
  region redraw is too slow to scrub smoothly, the player draws the already-sewn part once and adds
  stitches to it while playing forward. This is decided by measurement in item 1.
- **Two pictures of the same thing.** The panel and the canvas must not each build a plan. The
  plan and its worker move to shared app state in item 1, before the player bar needs them.
- **Editing under thread.** Without the vector art drawn, a filled object's edge is only visible
  as stitches. The selection outline and the underlay toggle cover this; if it is not enough, the
  underlay turns on while a drawing tool is active.
- **Merge conflicts.** STI-16 and STI-30 add Embroidery menu entries in their own branches.

## Not in this design

Fabric textures and garment mock-ups, a stitch list editor (E14), drag handles for entry and exit
points (E8), changing threads from the thread bar (E6), hiding illustration tools and menus, and
an embroidery home screen (the app shell design, decision 5).

## Docs to update when each item lands

`docs/embroidery-parity.md`: the rows Realistic preview, Stitch simulator, Travel through the
design and Needle points, connectors, function markers, the E5 gap row and the revision table.
`docs/gaps.md`: the Embroidery preview (E5) note. `docs/control-protocol.md` and `docs/mcp.md` for
`view.stitches` and the new `ui.stitchPreview` parameters. This file's status line. The app shell
design's decision 4, which this replaces.

## Sources

Behaviour of other products, from their public documentation and public write-ups, read on
2026-11-12. Described in our own words; no text, icon or artwork is copied.

- Wilcom EmbroideryStudio: workspace
  <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Basics/basics/basics-4.htm>, Stitch
  Player <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Basics/view/view-12.htm>,
  travel <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Basics/view/view-13.htm>,
  Color-Object List
  <https://docs.wilcom.com/embroiderystudio/e4/en/MainHelp/Basics/basics/Select_objects_with_Color-Object_List.htm>.
- Hatch Embroidery: features FAQ <https://hatchembroidery.com/faq/features>.
- Embrilliance: interface <https://embrilliance.com/Help/Platform%20Mac%201170/theinterface.htm>.
- Brother PE-Design 11: manual <https://support.brother.com/g/s/hf/htmldoc/ped/im/ped11/en/PED11_EN>.
- Embird Studio: main window
  <https://www.embird.net/machine-embroidery-tutorials/studio-next/mainwind.htm>, view
  <https://www.embird.net/manual/0800view.htm>.
- mySewnet: window parts
  <https://softwarehelp.mysewnet.com/MSW/140/Embroidery/Topics/Parts_of_the_Software_Window.htm>,
  Stitch Player <https://softwarehelp.mysewnet.com/MSW/120/StitchEditor/Topics/Stitch_Player.htm>.
- Bernina Embroidery Software 9: hooping
  <https://softwarehelp.bernina.com/en/MainHelp/PART_Processing/hooping/hooping-2.htm>.
- Ink/Stitch (documentation only; its code is GPL and was not opened): visualize
  <https://inkstitch.org/docs/visualize/>.

Unverified: hoop display and millimetre grid in Wilcom; the layouts of Janome Digitizer and Tajima
DG/ML by Pulse; whether any product marks out-of-hoop or dense areas on the canvas.

## Revision history

| Date | Change |
|---|---|
| 2026-11-12 | First version, proposed (STI-31). |
| 2026-11-12 | Items 3 and 4 implemented: what was built and what differs (figures on the thread bar, no control bar switch, no warning count). |
| 2026-11-12 | Item 2 implemented without the sequence link: what was built and why the link waits. |
| 2026-11-12 | Item 1 implemented: decisions updated with what was built (command parameters, region limits, transparency, where the state lives). |
