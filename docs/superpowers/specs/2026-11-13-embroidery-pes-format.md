# Embroidery: the PES and PEC machine files

Status: implemented 2026-11-13 (STI-9), measured by tests and a cross-check against pyembroidery
both ways. Not yet loaded on a machine.

PES is the file Brother (and Babylock, Bernina deco) home machines and PE-Design read. It wraps a PEC
block, which is what the machine sews, in a design section for editing software. A `.pec` file is the
PEC block alone. This is what `crates/embroidery/src/pes.rs` implements.

## Sources consulted

- pyembroidery 1.5.1, `PesWriter.py`, `PesReader.py`, `PecWriter.py`, `PecReader.py`,
  `PecGraphics.py` and `EmbThreadPec.py` (MIT, https://github.com/EmbroidePy/pyembroidery). Read for
  the layout; the 64-entry PEC thread palette is taken from `EmbThreadPec.py` as data
  (`pes/threads.rs`, attributed there and in NOTICE). Used as a tool for the cross-check. No code was
  ported; the thumbnail frame is drawn in code, not copied.
- EduTechWiki, "Embroidery format PEC": https://edutechwiki.unige.ch/en/Embroidery_format_PEC (header
  fields, the short and long move forms, jump and trim flags, `FE B0`, `FF`, thumbnails, palette).
- libpes wiki: https://github.com/frno7/libpes/wiki/PES-format,
  https://github.com/frno7/libpes/wiki/PEC-section (already a source of the parity document).
- The "redmean" colour distance: https://www.compuphase.com/cmetric.htm.

Nothing from a GPL project was opened. No machine, vendor software or Adobe software was used.

## PES (version 1, what pyembroidery writes by default)

| Offset | Content |
|---|---|
| 0 | `#PES0001` |
| 8 | u32: offset of the PEC block |
| 12 | i16 1 (scale to fit), i16 1 (hoop 130 × 180 mm), i16 1 (one object), `FF FF 00 00` |
| 22 | `CEmbOne` (i16 length 7, the name): 16 zero bytes, a 2 × 3 float transform `1 0 0 1 tx ty` placing the design in the hoop (`tx = 1000 − w/2`, `ty = 1000 + h/2`), i16 1, 0, 0, width, height, 8 zero bytes, i16 segment count, `FF FF 00 00` |
| — | `CSewSeg`: per segment `flag` (0 stitches, 1 a jump), palette entry, point count, points (i16 x from the left edge, y from the bottom edge, y down so ≤ 0); `03 80` between segments; then the colour log (count, then segment index and palette entry where the colour changes) and `00 00 00 00` |

Stitch runs of more than 32,767 points are split into segments. The reader skips all of this: it
follows the offset at 8 and reads the PEC block, for any version (`#PES0001` to `#PES0100`).
**Reading the design section (and the thread list of version 5 and later) is deferred**: colours come
from the PEC palette only.

## PEC block

| Offset | Content |
|---|---|
| 0 | `LA:`, the name (8 characters, padded to 16), `\r` |
| 20 | 12 spaces, `FF 00`, thumbnail stride 6, height 38 |
| 36 | 12 spaces |
| 48 | colour blocks − 1, then a palette entry (1–64) per colour block; spaces to 512 |
| 512 | stitch block: `00 00`, u24 length (from 512, the end byte included), `31 FF F0`, i16 width, i16 height, `E0 01`, `B0 01` |
| 528 | stitch records, then `FF` |
| — | thumbnails: the whole design, then one per colour block; 48 × 38 one-bit pixels, 228 bytes each |

### Stitch records (y down)

Each coordinate is one byte, −63 to 62 in seven bits, or two bytes: bit 0x80, a flag (0x10 jump,
0x20 trim) and twelve signed bits, so a move is at most ±2047. A stitch writes each coordinate short
when it fits; a jump writes both long with the jump flag; the first jump after a trim carries the
trim flag instead, and a trim with no jump after it is a trim of (0, 0). A colour change is `FE B0`
then 2, 1, 2, … in turn. The end is `FF`. The engine limits PES and PEC to 12.1 mm stitches and
jumps (Brother's published longest stitch is not confirmed; DST's limit is used).

### Colours

PEC stores no colours, only palette entries. Each distinct colour, in order of first use, takes the
nearest palette entry (redmean distance) that no earlier colour took, so two colours never share an
entry while entries remain; the design section uses the plain nearest entry. A plan's stop (PEC has
none) is a change to the same colour. On read, a block takes the palette thread (name, catalogue
number, brand Brother); an entry outside 1–64 takes the built-in colour list.

### Thumbnails

One bit per pixel, lowest bit leftmost (as pyembroidery writes; the wiki says most significant bit
first: **unverified on a machine**), a rounded frame drawn in code, the needle points scaled to fit
inside it.

## Recognising the files

By content: `#PES` (any version) or `#PEC0001`, whatever the file name. `.pes` and `.pec` export
rows write version 1 and the PEC file. Neither is on the `bai_mirror` panel list.

## Limits

Files over 64 MB, more than 10,000 colour changes or the plan step limit are refused; more than 255
colour blocks is an error on write. A PEC offset or length past the end of the file is held to it.

## Tests

- `crates/embroidery/src/pes.rs`: exact PEC header and stitch bytes of a tiny design, jump, trim and
  colour-change codes, a trim with no jump, a stop as a duplicate colour, the design section's
  segments and colour log, odd and truncated files, limits, the frame and thumbnails, and a property
  test: a random plan with jumps of up to 50 mm and trims, written as PES or PEC, reads back with
  identical needle points, jumps, trims, colour blocks and palette colours.
- `crates/embroidery/src/pes/threads.rs`: the palette, matching, unique entries.
- Engine tests (red–blue–red reads back as Brother Red, Ultramarine, Red) and fuzz cases
  (`pes_garbage_never_panics` with no header, a PEC header or a PES header).

## Cross-check with pyembroidery

Run 2026-11-13 with pyembroidery 1.5.1 from PyPI in a virtual environment, on two designs written
by `vectorcraft-cli run --commands` with `document.export`: the calibration design
(`stitch.calibration`, 3 colours, 392 needle points, 14 trims) and the lettering design
(`text.create {"x":40,"y":120,"text":"Hello","size":42.5}`, 1 colour, 603 needle points). Three
directions, by a script that reads our files with a small Rust program over `read_format` and with
`pyembroidery.read`, and compares needle points per colour block relative to the first stitch
(the steps are in `docs/development.md`, "Checking stitch files against another reader"):

- **Our file, pyembroidery reads it** (PES and PEC): needle points identical (392 and 603), colour
  changes 2 and 0, trims 14 and 1, palette colours identical.
- **pyembroidery writes the same plan** (version 1, its default): on the lettering design the PEC
  stitch block (1,231 bytes) and the whole PES design section (2,552 bytes) are byte for byte
  identical; the PEC header differs only in the name. On the calibration design the stitch blocks
  differ (917 against 895 bytes) where the plan jumps: pyembroidery writes every jump after the first
  as a trim and merges consecutive jumps into one move of up to 204.7 mm, and adds a stitch in place
  after a jump that moves on both axes; we keep a jump a jump (the trim flag only after a plan trim)
  and write the plan's jumps as they are (at most 12.1 mm each). The design sections agree up to the
  first such jump. Thumbnails differ in their frame (ours is drawn in code), not in size or count.
- **pyembroidery's file, our reader reads it**: needle points and colour changes identical to
  pyembroidery's reading and to the plan; trims identical to pyembroidery's reading (21 on the
  calibration design, because its writer made every jump a trim).

Not verified by this: what a Brother machine does with the file, the thumbnail bit order, the
design section in PE-Design.

## Not done

- Whether a Brother machine accepts the file: untested.
- PES versions above 1 (thread lists, metadata) are not written; their design sections are not read.
- JEF and VP3 (the rest of the "PES/PEC, JEF, VP3" row) remain.
