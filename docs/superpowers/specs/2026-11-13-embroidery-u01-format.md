# Embroidery: the U01 machine file

Status: implemented 2026-11-13 (STI-9), measured by tests and a cross-check against pyembroidery
both ways. Not yet loaded on a machine.

U01 (also written `.u??`, "FDR") is the file Barudan machines read; the owner's BAI Mirror family
shares the Barudan record layout (see the [DSB spec](2026-11-11-embroidery-dsb-format.md)). U01 adds
what DSB lacks in our writer: the needle number of each colour block and a binary header. There is no
public specification from Barudan. This is what `crates/embroidery/src/u01.rs` implements.

## Sources consulted

- pyembroidery 1.5.1, `U01Reader.py` and `U01Writer.py` (MIT, https://github.com/EmbroidePy/pyembroidery).
  Read for the record layout, the control codes and the header fields; used as a tool for the
  cross-check below. No code was ported.
- EduTechWiki, "Embroidery format U??": https://edutechwiki.unige.ch/en/Embroidery_format_U%3F%3F
  (header parts A and B, the 5-bit function codes, the count field, teach blocks in machine-edited
  files).
- Barudan BEVT instruction manual listing (search result only; its function-code appendix was not
  readable): https://manualmachine.com/barudan/bevt/19718449-instruction-manual.

Nothing from a GPL project was opened. No machine, vendor software or Adobe software was used.

## Layout

A 256-byte header, then records of three bytes until an end record. Units are 0.1 mm, y up in the
file (the reader and writer flip to the plan's y down).

### Header (0x000–0x0FF)

| Offset | Size | Field | We write | Source |
|---|---|---|---|---|
| 0x00 | 128 | ASCII `0` (0x30) | 128 × `0` | wiki part A, pyembroidery |
| 0x80 | i16 ×4 | left, bottom, right, top extents, y up, from the origin | from every record's position, the origin included | wiki part B, pyembroidery |
| 0x88 | u32 | unknown, 0 in files seen | 0 | wiki, pyembroidery |
| 0x8C | u32 | record count | the records written, the end record included (× 3 is the body length, as the wiki says) | wiki; pyembroidery writes one more |
| 0x90 | i16 ×2 | end point, y up | where the last record leaves the needle | wiki, pyembroidery |
| 0x94 | 108 | zero (machine-edited files keep teach data here) | zeros | wiki |

The reader trusts none of it: it skips 256 bytes.

### Records: `[control, |dy|, |dx|]`

Bit 0x80 is set on every record. Bit 0x20 is set when the x move is **zero or negative** (left),
bit 0x40 when the y move is zero or negative (down, y up). That rule is why the codes without a move
read 0xE7, 0xE8, 0xE9…: both sign bits are set. The low five bits say what the record is:

| low bits | code | meaning | written |
|---|---|---|---|
| 0x00 | 0x80–0xE0 | stitch | yes |
| 0x01 | 0x81–0xE1 | jump | yes |
| 0x02, 0x04 | | fast, slow stitch | no; read as a stitch |
| 0x03, 0x05 | | fast, slow jump | no; read as a jump |
| 0x06 | 0xE6 | T1, top thread trim | no; read as a trim |
| 0x07 | 0xE7 | T2, trim | yes |
| 0x08 | 0xE8 | C00, stop | yes, for a plan's stop |
| 0x09–0x17 | 0xE9–0xF7 | C01–C15, change to needle 1–15 | yes |
| 0x18 | 0xF8 | end | yes, `F8 00 00` |

Each axis is one unsigned byte, so a record holds a move of up to 255 units; the writer writes at
most 127 (12.7 mm): the engine limits stitches and jumps to that, as for DSB, and the writer splits
its own travel (to the first stitch, back to the origin) into jumps of at most 127, as pyembroidery
does. The reader takes the full 255. The wiki's second table names 0x13–0x16 groups and 0x17 a frame code;
pyembroidery reads 0x09–0x17 as the fifteen needles, which a 15-needle machine needs, and so do we.
Any other code ends the file with "an unknown code ended the file early"; a move on a trim, stop or
needle record is applied as travel.

## Needles

The file sets the first block's needle before anything else, and each colour change is a change to
the next block's needle. `WriteOptions::needles` carries the needle per block (1–15); the engine
fills it from the machine's needle assignment (`stitch.needles`): each distinct colour, in sewing
order, takes the next needle, and a colour sewn again goes back to its needle. Colours past the
machine's needles, and a plan written without the list, take block `i % 15 + 1`, as DSB writes.

On read, a needle change starts a new block when the current block has stitches, else it names the
needle of the current block. `Input::needles` returns the needle of each block; the format stores
no colour, so a block's colour is the built-in colour of its needle number (blocks on one needle
share a colour).

**The block model is not extended.** `vectorcraft_stitch::Block` has no needle field: needles
travel beside the plan, in `WriteOptions::needles` and `Input::needles`, because the document an
opened file becomes has nowhere to keep a needle per group either, and a field on `Block` would touch
every plan builder in the stitch crate. When the document gains per-group needles, they can move into
the block.

## What we write

- **Start and end** as DSB: the origin is the centre of the stitches, the first needle point or the
  design start; a leading jump reaches the first stitch; the file may end with a trim and travel back
  to the origin (`returnToStart`).
- **Trims**: one `0xE7` record; `jumpsPerTrim` does not apply on write or read. No trim is added
  before a needle change (pyembroidery adds none either, and the DSB file the BAI Mirror accepts has
  none); `trimBeforeColorChange` adds one.
- **Stops**: `0xE8` with the thread kept, in the same block.
- **Long stitches**: split by `prepare` into equal jumps and a stitch, read back as jumps and a stitch.

U01 is recognised by its `.u01` extension only. It is not on the `bai_mirror` profile's list of
panel formats: no public page says the panel reads it, so an export for that machine warns.

## Limits

Files over 64 MB, more than 10,000 colour blocks and more than the plan step limit are refused. A
truncated record or a missing end record is a warning. A plan move beyond 127 on write is an error.

## Tests

- `crates/embroidery/src/u01.rs`: exact bytes of a tiny design (header and records), sign and control
  codes, needles from the options and in turn, stops, odd and truncated files, fast/slow/T1 records,
  limits, and a property test: a random plan with trims and stops, from the centre or the first
  stitch, with random needles, reads back with identical needle points, jumps, trims, stops, colour
  blocks and needles.
- `crates/engine/src/cmd/fileio/tests_stitchfile.rs`: export and reopen, the calibration design
  re-exported with the same needle points, a red–blue–red design written with needles 1, 2, 1, the
  format row, the `bai_mirror` warning.
- `crates/engine/tests/import_fuzz.rs`: `u01_garbage_never_panics` and the mutated-file case.

## Cross-check with pyembroidery

Run 2026-11-13 with pyembroidery 1.5.1 from PyPI in a virtual environment, on two designs written
by `vectorcraft-cli run --commands` with `document.export`: the calibration design
(`stitch.calibration`, 3 colours, 392 needle points, 14 trims) and the lettering design
(`text.create {"x":40,"y":120,"text":"Hello","size":42.5}`, 1 colour, 603 needle points). Three
directions, by a script that reads our files with a small Rust program over `read_format` and with
`pyembroidery.read`, and compares needle points per colour block relative to the first stitch
(the steps are in `docs/development.md`, "Checking stitch files against another reader"):

- **Our file, pyembroidery reads it**: needle points identical on both designs (392 and 603),
  colour changes 2 and 0, trims 14 and 1, needles 1, 2, 3 and 1, on both sides.
- **pyembroidery writes the same plan** (built from our reading, with `needle_change` for each
  block): the lettering design's 608 records are byte for byte identical; the calibration design's
  440 records are identical but for one long stitch split into seven pieces, where the two writers
  place the one-unit rounding remainder in a different piece (same needle points). The header is
  identical but for the record count at 0x8C: pyembroidery writes one more than the records it
  writes (441 for 440); we write the count the wiki describes (records × 3 is the body length).
  Before the cross-check our writer split travel into moves of up to 255 units; it now splits at 127,
  as pyembroidery does, and the byte comparison above is after that change.
- **pyembroidery's file, our reader reads it**: needle points, colour changes, trims and needles
  identical to pyembroidery's own reading, and to the plan.

Not verified by this: anything a machine does with the file; the header extents' sign convention
beyond agreeing with pyembroidery; fast/slow and T1 records (neither writer writes them).

## Not done

- Whether a Barudan or BAI machine accepts the file: untested.
- Fast and slow (speed) records, T1, groups and teach blocks are read past or ignored, never written.
