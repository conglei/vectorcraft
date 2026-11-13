# Embroidery: the TBF machine file

Status: implemented 2026-11-13 (STI-9), measured by tests and a cross-check against pyembroidery
both ways. Not yet loaded on a machine.

TBF ("Tajima binary format") is the file Tajima machines on a LAN or TFD read. Unlike DST it
addresses needles: the header lists the needle of each change and the colour of each needle. There
is no public specification from Tajima. This is what `crates/embroidery/src/tbf.rs` implements.

## Sources consulted

- pyembroidery 1.5.1, `TbfReader.py` and `TbfWriter.py` (MIT, https://github.com/EmbroidePy/pyembroidery).
  Read for the header layout (offsets 0x83, 0x10A, 0x20E, 0x376, 0x600), the record layout and the
  control codes; used as a tool for the cross-check below. No code was ported.
- Wilcom on Tajima machines, "TBF … holds needle addressing information": https://wilcom.com/tajima
  (search result).
- Our DST header code (`dst/header.rs`) for the DST-style fields.

No public page describes the `TP` and `JC` fields; we write what pyembroidery writes. Nothing from a
GPL project was opened. No machine, vendor software or Adobe software was used.

## Layout

A 1,536-byte (0x600) header, then three-byte records, then 0x1A. Units are 0.1 mm, y up in the file.

### Header

| Offset | Content |
|---|---|
| 0x000 | `3.00`, then spaces to 0x80 |
| 0x080 | DST-style fields, each ending in `\r`: `LA:` name (16), `ST:` records (7, the end record included), `CO:` needle changes (3, the first needle set included), `+X:` `-X:` `+Y:` `-Y:` extents (5 each, as DST: distances from the origin, y up), `AX:` `AY:` end point (sign and 5) |
| 0x0DE | `TP:` 32 characters (`EG/` and spaces; meaning unknown) and `JC:3` (meaning unknown) |
| 0x107 | `DO:`, then 256 bytes at 0x10A: the needle of each needle change in order, zero after the last |
| 0x20A | `\r`, `DA:`, then at 0x20E five bytes per needle: `E` (0x45), red, green, blue, space; at most 72 |
| — | spaces to 0x376, `\r`, 0x1A, spaces to 0x600 |

### Records: `[dx, dy, control]`

`dx` and `dy` are signed bytes (y up), so a move is at most 127 per axis.

| control | meaning | written |
|---|---|---|
| 0x80 | stitch | yes |
| 0x90 | jump; a jump of (0, 0) is a trim | yes, never of (0, 0) |
| 0x86 | trim | yes |
| 0x81 | change to the next needle of `DO` (needle 0 there: a stop) | yes |
| 0x40 | stop | yes, for a plan's stop |
| 0x8F | end, followed by the byte 0x1A | yes |

Moves on needle, stop and trim records are written as zero and ignored on read, as pyembroidery
reads them. Any other code ends the file with "an unknown code ended the file early".

## Needles and colours

As U01 (see [the U01 spec](2026-11-13-embroidery-u01-format.md), "Needles"): the first record sets
the first block's needle, each colour change is a needle change, and `WriteOptions::needles`, filled
from the machine's needle assignment, names them; `DO` lists them in order. More than 256 needle
changes is an error ("a TBF file holds at most 256 needle changes").

`DA` is read as **the colour of each needle**, needle 1 first: we write, for each needle the plan
uses up to the highest, the colour of the first block sewn with it (a needle the plan skips gets the
built-in colour of its number), and on read a block takes the colour of its needle. pyembroidery
instead reads `DA` as one colour per block in order and writes its thread list that way; the two
agree when no colour is sewn twice. Which reading the machine takes is **unverified**; pyembroidery's
own note says the list holds "any threads in the set", which fits a per-needle reading.

## What we write

- **A trim before every needle change** when the thread is attached (stitches since the last trim),
  as pyembroidery writes (it marks TBF as a format of explicit trims); `trimBeforeColorChange` makes
  no difference beyond that.
- Start, end, stops and long stitches as U01; no zero-length jump (it would read as a trim).

TBF is recognised by its `.tbf` extension (its `3.00` start is not a signature anyone documents). It
is not on the `bai_mirror` profile's panel list, so an export for that machine warns.

## Limits

As U01; a needle number above 15 in `DO` ends the file with a warning.

## Tests

- `crates/embroidery/src/tbf.rs`: exact header and record bytes of a tiny design, a trim before each
  needle change, `DO` and `DA` contents, stops, odd and truncated files (no `DO:`, needle 0, a needle
  over 15, a jump of nothing), limits, and a property test: a random plan with trims and stops reads
  back with identical needle points, jumps, stops, colour blocks and needles (the trims TBF adds
  before needle changes aside).
- Engine tests and fuzz cases as for U01 (`tbf_garbage_never_panics`).

## Cross-check with pyembroidery

Run 2026-11-13 with pyembroidery 1.5.1 from PyPI in a virtual environment, on two designs written
by `vectorcraft-cli run --commands` with `document.export`: the calibration design
(`stitch.calibration`, 3 colours, 392 needle points, 14 trims) and the lettering design
(`text.create {"x":40,"y":120,"text":"Hello","size":42.5}`, 1 colour, 603 needle points). Three
directions, by a script that reads our files with a small Rust program over `read_format` and with
`pyembroidery.read`, and compares needle points per colour block relative to the first stitch
(the steps are in `docs/development.md`, "Checking stitch files against another reader"):

- **Our file, pyembroidery reads it**: needle points identical (392 and 603), colour changes 2 and
  0, trims 16 and 1 (two trims before the needle changes), needles 1, 2, 3 and 1, colours identical
  (pyembroidery's per-block reading of `DA` agrees with ours when no colour repeats).
- **pyembroidery writes the same plan**: the lettering design's 610 records are byte for byte
  identical, and so is the calibration design's but for the same split-stitch rounding as U01 (two
  records swapped). The header is identical but for the name (the script names its pattern) and, on
  the lettering design, the `+Y` and `-Y` values: pyembroidery writes the downward extent as `+Y`, as
  its DST writer does; we write the upward extent as `+Y`, as our DST header does (one unit apart
  here, equal on the calibration design, which is centred).
- **pyembroidery's file, our reader reads it**: needle points, colour changes, trims and needles
  identical to pyembroidery's reading and to the plan.

Not verified by this: what a Tajima machine does with `TP`, `JC` and the per-needle `DA`.

## Not done

- Whether a Tajima machine accepts the file, and what `TP`, `JC` and the per-needle `DA` mean to it:
  untested.
- The `.ct0` companion file pyembroidery can write is not written.
