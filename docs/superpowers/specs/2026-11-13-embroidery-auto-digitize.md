# Auto-digitize artwork, first pass (gap E10, STI-24)

> **Status:** first pass implemented 2026-11-13, measured by tests and in the running app; not yet
> sewn at a machine. Numbers marked *estimated* are starting points to check at the stitch-out.
> **Change:** new spec.

Until now a shape became stitches by its stitch type alone: Automatic sews a fill as tatami and a
stroke as running stitch, whatever the shape looks like. A 4 mm bar came out as a tatami patch, a
hairline as a fill nobody can see. Digitizers choose the stitch type by the shape's width. This
pass makes that choice for the user: click a shape, or convert a whole image, and get editable
stitch objects (columns, fills, runs) with their properties set. Clean room: behaviour from public
digitizing guides and Ink/Stitch's documentation (GPL, read as documentation only, no code looked
at); the algorithms are the lettering pass's (E9) and textbook morphology.

## Sources for the thresholds

| Rule | Default | Sources |
|------|---------|---------|
| Narrower than the minimum column: running stitch | 1 mm (`minColumn`) | Graphics Pro, "areas under 1 mm wide are usually done with running stitch" (<https://graphics-pro.com/?p=95539>); 1 mm the least satin (<https://hoopingstation.com/blogs/articles/run-satin-and-fill-the-only-3-stitch-types-you-need-to-digitize-and-stitch-almost-anything>) |
| Satin up to the widest satin, wider: fill | 8 mm (`maxSatin`) | Wilcom about 10 mm before snags (<https://wilcom.com/resources/blog/embroidery-stitch-techniques>); mySewnet object types and large lettering (<https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Overview_of_Object_Types.htm>, <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Large_Lettering.htm>); 8 mm as lettering uses |
| A stroke becomes a satin column the stroke weight wide | stroke weight | Ink/Stitch Stroke to Satin (<https://inkstitch.org/docs/satin-tools/>) |
| Leftover stretch shorter than half the widest satin is filled with the wide part | 4 mm | *estimated* |
| Compact blob (centre lines shorter than 2 × its width, at least half the widest satin wide) is filled | — | *estimated* |
| A column reaches 0.3 mm into the fill it meets | 0.3 mm | *estimated*; lettering uses 0.2 mm between columns |

Sources disagree on the widest satin (7, 8, 10, 12 mm); 8 mm is in the middle, the lettering
default, and a parameter.

## Classification (`vectorcraft-stitch::autodigitize`)

Shared with lettering: the outline is rasterised at max(0.05 mm, size/800), its distance transform
and Zhang–Suen skeleton make a graph of strokes (`skeleton.rs`), each stroke gets a smoothed centre
line and rails cast to the vector outline (`columns.rs`, moved out of `lettering.rs`). The graph
now also merges every node where two edges meet and run straight on (within 45°): a ring's
staircase left dozens of such nodes, so an O was 48 short columns; corners stay nodes so a column
never turns a sharp corner.

1. **Wide part.** The morphological opening of the shape by a disc `maxSatin` across: pixels within
   `maxSatin/2` of a pixel whose half width is at least `maxSatin/2` (two distance transforms).
2. **Stretches.** Each stroke is cut where it enters the wide part (less the 0.3 mm overlap). A
   leftover stretch shorter than `0.5 × maxSatin` is dropped and its area joins the fill (the
   corners of a square, a fillet); a longer one (a tail, an arm) is cut into satin columns and runs:
   a point is a column point when its width as drawn is more than `minColumn` plus one pixel.
3. **Whole shape.** No leftover stretch: one tatami fill with the outline as drawn (holes kept). No
   wide part: columns and runs, unless the shape is a compact blob (fill). Fill and leftovers: the
   fill's outline is the wide part plus the dropped leftovers, traced from the pixel mask (crack
   following, edge midpoints, Douglas–Peucker at 0.75 pixel), and the columns overlap it.
4. **Open stroked paths** are classed by their stroke weight: running stitch up to `minColumn`, a
   satin column up to `maxSatin`, else the stroke's outline is filled.

Every column is at least `minColumn` wide as drawn (property test on random polygons).

## Commands

- `stitch.autoDigitize {ids?, minColumn?, maxSatin?, keepSource?}`: replaces each selected path,
  compound path or group's paths (type goes through `text.toStitches`, so it is left alone) with a
  group named after it holding fill, then satin columns, then runs, each a stitch object in the
  source's thread (fill colour, else stroke colour), with stitch type and parameters set (satin
  columns: two open rails, type Satin; fills: closed outline, type Tatami; runs: open path, type
  Run). The new objects take the source's place in the sewing order. One undo step. `keepSource`
  hides the source instead of removing it. → `{created, columns, fills, runs}`.
- `stitch.autoDigitize.info` (query): what each selected shape would become, nothing changed.
- `image.autoDigitize {id?, colors? (2–12, default 6), minRegionMm2? (default 2), widthMm?,
  minColumn?, maxSatin?, keepImage?}`: Image Trace with an embroidery preset (Color mode, limited
  palette, white ignored, fills only, no strokes, noise from `minRegionMm2`), every traced region
  classified as above, then Sort by Colour and Closest Join from the sequencing pass. The image
  moves to a hidden template layer (`keepImage`, default) or is removed. One undo step.
- Embroidery › Auto-Digitize (`tool.autoDigitize`): a click tool that runs `stitch.autoDigitize` on
  the object under the pointer; Enter applies it to the selection. Embroidery › Auto-Digitize
  Image… opens a dialog with the colours, smallest region and widest satin, and shows the result.

## Not in this pass

- Fill angle from the shape's main direction, and turning satin at sharp corners (corners stay
  separate columns).
- Overlap and underlay between neighbouring colour regions of a traced image (regions abut).
- Branching order inside a mixed shape: objects are sewn fill, columns, runs; Closest Join chooses
  their ends.
- MCP tools: the commands reach MCP through `engine.execute`; dedicated tools are later work.
