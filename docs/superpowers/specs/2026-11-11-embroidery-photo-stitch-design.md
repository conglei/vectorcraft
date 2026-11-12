# Embroidery photo stitch: design

> **Status:** first pass implemented · **Date:** 2026-11-11 · **Covers:** gap E11 in [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-17): colour photo stitch, the direction field from the photo, a direction-following fill with varied stitch length and feathered edges, `photo.palette`, `photo.regions`, `photo.directionField`, `photo.toStitches`, `photo.syntheticFur`, the MCP tool `photo_to_stitches`

## Goal

A prepared pet photo becomes a fill design whose rows follow the way the fur lies. Digitizers
place fill directions by hand today; here the direction comes from the photo itself, and the
regions stay ordinary, editable art.

Not in this pass: single-colour photo modes, the eye-and-nose detail pass, a dialog in the app (the
commands and the MCP tool are the interface), and a real stitch-out.

## Pipeline

```
photo ─► prepare (E12: none | nomodel | gemini) ─► scale to maxSide
      ─► palette: blur, quantize to 7–16 colours, match each to a thread (CIE76)
      ─► regions: trace (abutting, small patches merged, outlines simplified)
      ─► direction field: structure tensor, one direction per cell, with coherence
      ─► fills: per region, rows along the field (or tatami at the region's mean direction)
      ─► sequence: colour by colour, the colour covering most first
      ─► document: needle points over a template layer of the regions (or editable regions)
      ─► document.export {format: dst}
```

| Stage | Command | In | Out | Caps and parameters |
|---|---|---|---|---|
| Photo | all | `file` (PNG, JPEG, WebP, GIF, TIFF, BMP) or `image` (base64) | RGBA raster | 64 MB file, 192 megapixels decoded (scaled to 64), then `maxSide` 600 px (64–2048, never scaled up) |
| Prepare | `photo.toStitches` (`photo.prepare` alone) | photo | prepared photo | `provider`: `none`, `nomodel`, `gemini`; not given: the `photoProvider` preference. Gemini runs only when named or chosen there: the only stage that sends anything off this computer. Clean-up is off here, so the fur texture reaches the field |
| Palette | `photo.palette` | worked photo | thread colours with coverage | `colors` 8 (7–16), `blur` 1.5 px (0–8: softens single hairs before colours are chosen), `chart` (default the active chart, else the starter set). Image Trace's Limited colour quantization (median cut, then weighted k-means in Oklab), each colour matched to the nearest chart thread by CIE76; two photo colours matching one thread become one thread colour |
| Regions | `photo.regions` | worked photo | filled paths, a group per thread colour, each with `StitchProps { type: tatami, fillAngle }` | `minArea` 4 mm² (0–10 000: smaller patches merge into their neighbour, Image Trace's noise step), `detail` 40 (0–100: Image Trace's Paths fidelity), at most 200 000 anchor points and 4 000 regions, else an error asking for a larger `minArea` |
| Field | `photo.directionField` | worked photo | grid of angles and coherence, JSON; PNG preview | `cell` 2 mm (0.5–50), `smoothing` 2 mm (0.2–100); worked at most 1024 px a side, at most 262 144 cells |
| Fills | `photo.toStitches` | region outlines (mm), field (mm) | runs of needle points | `spacing` 0.4 mm (0.2–5), `minLength` 2 / `maxLength` 4 mm (0.5–12), `feather` 0 mm (0–10), `featherEdges` both / start / end, `seed` 1, `maxStitches` 100 000 (up to 1 000 000); 4 000 000 traced points per region, 16 M index cells |
| Sequence and document | `photo.toStitches` | runs per region | a new document | colours by coverage, most first; regions in trace order; runs joined by trim and jump; lock stitches and travel trims added by `prepare` for the commercial 15-needle profile |

Every stage is a command with documented params, `enabled` always (nothing needs an open
document), and errors that say which param to change. Sizes are mm on the page: `widthMm` 100
(10–1000), `heightMm` optional (with both, the design fits within them).

## Direction field

The gradient structure tensor (second-moment matrix):

1. The photo is composited onto white, turned grey (Rec. 601 luma) and box-averaged down by a
   whole factor until its longest side is at most 1024 px.
2. A light Gaussian (σ 1 px of the source) against pixel noise; gradients by central differences.
3. Per pixel `J = [gx², gx·gy; gx·gy, gy²]`, each entry smoothed by a Gaussian of the integration
   scale (`smoothing`, capped at σ 48 working pixels). Averaging the tensor rather than the angles
   is what keeps the two sides of a dark hair (opposite gradients) from cancelling.
4. Per cell, `J` averaged over the cell. The dominant gradient direction is
   `φ = ½ atan2(2Jxy, Jxx − Jyy)`; the texture runs at right angles to it. Coherence is
   `((λ1 − λ2)/(λ1 + λ2))²` with `λ1 − λ2 = √((Jxx − Jyy)² + 4Jxy²)` and `λ1 + λ2 = Jxx + Jyy`; a
   flat cell has coherence 0.
5. Angles are reported in degrees counter-clockwise from horizontal as seen, `[0, 180)`, the
   convention of `fillAngle`.

Sampling between cells blends doubled angles (`(cos 2a, sin 2a)`) weighted by coherence, so a
direction and its opposite agree and unclear cells count little. Coherence-enhancing diffusion
(Weickert) would regularise the field further; it is not needed for the first pass, where the
integration scale and the coherence weighting are enough on the test picture.

Placement: `crates/trace` (`vectorcraft-trace::direction_field`, L2). It is raster analysis like
the rest of that crate, shares its `Raster` and size limits, and the region stage already uses the
crate; a crate of its own would add a layer entry for one module.

## Direction-following fill

`vectorcraft-stitch::field_fill` (`crates/stitch/src/fill_field.rs`, L2, std only, no threads or
clock), on the tatami fill's region machinery (`Region::inside`, `Region::clear`, long-stitch
splitting, plan rounding):

- **Rows** are evenly spaced streamlines of the field (Jobard and Lefer 1997). A row is traced from
  a seed both ways by midpoint steps of half the spacing (at most 0.5 mm); each step keeps the sign
  of the last (the field has no sign). It stops at the outline (found by bisection), at a turn
  sharper than 35° in one step, or within half the spacing of a row already placed (a cell index of
  the points). Seeds are tried one spacing to either side of each row, about every spacing along
  it; a scan of the region at the spacing seeds any part they never reach (islands, the far side of
  a hole).
- **Weak field.** The region's mean direction (coherence-weighted, over the cells inside it) is
  blended in with weight 0.05, so where the photo has no clear direction the rows run straight at
  that angle, as tatami would.
- **Stitches.** Each row is cut into `n` stitches of random length within `minLength–maxLength`,
  measured along the row: `n` is the count nearest to the mean length that fits the range; lengths
  start equal and neighbouring pairs trade a random amount within the range. A row that fits no
  whole count is shortened at its end; a row shorter than `minLength` is dropped. The generator is
  SplitMix64, seeded per region from `seed`, so the same input sews the same.
- **Feathered edges.** With `feather`, each row end pulls in by a random amount up to `feather`
  (never below one stitch): a ragged fur edge, as feathered satin and random edge prolongation do
  in the public documentation of other digitizing packages. `start` and `end` are the row's ends
  going along the region's direction as given.
- **Order.** Rows are sewn nearest end first (a cell index of row ends). The needle travels inside
  the region from one row to the next when the straight way is clear and at most 5 mm; otherwise
  the run ends, and the plan trims and jumps.

## Document and export

`photo.toStitches` opens a new document titled "Photo stitches", in millimetres, the artboard the
design plus 10 mm:

- `fill: field` (default): the plan, prepared for the commercial 15-needle profile (lock stitches,
  travel trims), as `to_document` makes it: a `Stitches` layer whose paths are the needle points,
  which `document.export` and `stitch.info` take anchor by anchor and export unchanged. Below it a
  template layer, `Photo regions`, holds the regions (shown dimmed, never sewn).
- `fill: tatami`: the regions on an ordinary layer, sewn by their stitch properties (tatami at the
  region's mean direction, the spacing and the mean stitch length given), so they can be edited
  and re-sewn.

The command reports the counts the export would write (`stitch.info` on the new document).

## Test picture

No photo is committed. `vectorcraft-imageprep::synthetic_fur` draws one in code: the left half warm
brown hair at 30°, the right half grey-brown hair at 120° (streaks of smooth value noise along the
hair), and a light eye disc on the right. `synthetic_fur_angle` gives the known direction at a
point. `photo.syntheticFur {path}` writes it for trials.

## Acceptance

`crates/engine/tests/photo_stitch.rs`: the synthetic picture through `photo.toStitches` and
`document.export {format: dst}`, under 100 000 stitches; per half, the length-weighted mean
direction of the plan's stitches within 15° of the known angle; `stitch.preview` renders it.

Command line, from a photo to a DST:

```sh
vectorcraft-cli run \
  --cmd photo.syntheticFur --params '{"path": "/tmp/fur.png"}' \
  --cmd photo.toStitches --params '{"file": "/tmp/fur.png", "provider": "none", "widthMm": 80, "feather": 1}' \
  --cmd document.export --params '{"path": "/tmp/fur.dst", "format": "dst"}'
```

## Sources

- M. Kass, A. Witkin, *Analyzing oriented patterns*, Computer Vision, Graphics, and Image Processing
  37 (3), 362–385, 1987, doi:10.1016/0734-189X(87)90043-0.
- J. Bigün, G. Granlund, *Optimal orientation detection of linear symmetry*, ICCV 1987.
- J. Weickert, *Coherence-enhancing diffusion filtering*, IJCV 31, 1999 (considered, not used).
- B. Jobard, W. Lefer, *Creating evenly-spaced streamlines of arbitrary density*, Visualization in
  Scientific Computing 1997.
- Structure tensor and coherence, summary: <https://en.wikipedia.org/wiki/Structure_tensor>.
- Fill rows that follow a guide line, user documentation only:
  <https://inkstitch.org/docs/stitches/guided-fill/> (no code read; the project is GPL).
- Feathered edges and random stitch length in public user documentation:
  <https://softwarehelp.mysewnet.com/MSM/140/Embroidery/Topics/Feathered_Satin.htm>,
  <https://embird.net/machine-embroidery-tutorials/studio-next/par_fill.htm>.
- Tatami fills and fill patterns: <https://edutechwiki.unige.ch/en/Tatami_embroidery_fill_patterns>.

No code was copied; every piece above is written from the papers and the behaviour the
documentation describes.

## Deferred

- Single-colour photo modes (rows of varying spacing from tone; an open meander).
- The eye-and-nose detail pass (satin or run detail over the fills).
- Layered fills (a lighter layer over a darker one), underlay under the direction fill, pull
  compensation.
- A dialog in the app; the field as an overlay on the canvas.
- Coherence-enhancing diffusion of the field; following region edges where the field is weak.
- A real stitch-out to calibrate spacing and lengths.
