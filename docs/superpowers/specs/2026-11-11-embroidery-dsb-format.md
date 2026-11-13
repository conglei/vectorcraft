# Embroidery: the DSB machine file

Status: first slice implemented 2026-11-11 (STI-9), measured by tests and a cross-check against
pyembroidery. 2026-11-13 (STI-37): the owner's BAI Mirror rejected our file ("no valid stitches");
stitch and jump records now carry bit 0x80 like every other code of the format. Not yet stitched.

DSB is the file Barudan machines read. It has no public specification. This is what the reader and
writer in `crates/embroidery/src/dsb.rs` implement, taken from the two open implementations below.

## Sources consulted

- pyembroidery 1.5.1, `DsbReader.py` and `DstReader.py` (MIT, https://github.com/EmbroidePy/pyembroidery).
  Read for the record layout and control codes. It has no DSB writer.
- libembroidery, `format-dsb.c` (zlib, https://github.com/Embroidermodder/libembroidery; the
  repository's licence is zlib, the file has no header of its own). Read for the sign bits and the
  alternative end codes. Its writer is an unfinished stub.
- Our own DST code (`dst.rs`, `dst/header.rs`) for the header.

No code or table was ported; the constants are facts of the format. Nothing from a GPL project was
opened. No machine or vendor software was used.

## Layout

A 512-byte header, then records of three bytes until an end record. Units are 0.1 mm. File
orientation is y up (the reader and writer flip to the plan's y down).

### Header

The DST header (`LA:` name, `ST:` record count, `CO:` colour changes, extents, `PD:`), padded with
spaces to 512 bytes, written by the shared `dst::header`. Both pyembroidery and libembroidery
read a header this way or skip it; our reader takes only the name and `TC:` thread colours and
does not trust the counts and extents. Whether a machine needs any particular header field is not
known; unknown.

### Records: `[control, |dy|, |dx|]`

| control | meaning |
|---|---|
| `0x80` | stitch (bit 0x80 is set on every record; a reader masks the low five bits, so `0x00` reads the same) |
| `0x81` | jump (move without stitching) |
| `+ 0x20` | on a stitch or jump: dx is negative (left) |
| `+ 0x40` | on a stitch or jump: dy is negative (down, y up) |
| `0xE7` | trim (move bytes ignored; we write 0, 0) |
| `0xE8` | stop (thread change, same needle) |
| `0xE9`..`0xF7` | change to needle 1..15 (`0xE8 + n`) |
| `0xF8` | end of file |
| `0x91`, `0x87` | also read as end (libembroidery) |

Each axis is one unsigned byte, so a move is at most 255 units. A stitch or jump is a record whose
low five bits are 0 or 1 (pyembroidery); other bits are ignored on read. Any other control byte
ends the file with the warning "an unknown code ended the file early". Source: pyembroidery
`DsbReader.py` (layout, codes 0xE7, 0xE8, 0xE9 to 0xF7, 0xF8), libembroidery (sign bits agree; the
extra end codes).

## What we write

- Moves: at most 255 per axis in the format, but the engine limits a DSB export to 127 units
  (12.7 mm), as for EXP, because that is the longest stitch the machines take; `maxStitch` defaults
  to 12.7 mm for DSB.
- **Trims**: DSB has a trim code, so a trim is one `0xE7` record, as in EXP. `jumpsPerTrim` does not
  apply on write and the reader never reads a run of jumps as a trim.
- **Long stitches** (STI-27): a stitch longer than the limit is split by `prepare` into equal jumps
  followed by an equal stitch. With no jump-count convention in DSB, these are read back as jumps
  and a stitch: the needle points are identical; the jumps stay jumps.
- **Colour changes**: a change to the next needle, `0xE8 + (n mod 15) + 1` where `n` counts the
  changes, so the first change is needle 2 (`0xEA`) and the sixteenth wraps to needle 2 again. A
  plan's stop is written as a colour change, as in DST. On read, the stop and every needle change
  start a new block (the needle number is not kept; colours come from the header's `TC:` lines or the
  built-in palette).
- **Start and end**: the file starts at the centre of the stitches or the first needle point
  (option `start`), with a leading jump to the first stitch when it is not at the origin, and may
  end with travel back to the origin (`returnToStart`), preceded by a trim when the thread is
  attached, as DST does. The end record is `0xF8, 0, 0`.
- **Header**: counts (`ST` includes the end record), extents and end point as DST writes them.

DSB is recognised by its `.dsb` extension only (there is no signature, and the DST-style header is
not unique to it), as EXP is. When a file with the DST-style header is named `.dsb`, `document.open`
reads it as DSB, not DST; any other name reads it as DST. DSB is not offered in the Stitch File
Options dialog yet (Export As → DSB writes it with the default options).

## Limits

Files over 64 MB, more than 10,000 colour blocks and more than the plan step limit are refused.
A truncated record or missing end record is a warning. A move beyond 255 on write is an error, never
a dropped record.

## Tests

- `crates/embroidery/src/dsb.rs`: exact bytes of a tiny design, sign and control codes, needle
  wrap, odd and truncated files, the end codes, the range error, the block limit, and a property
  test that a random plan round-trips through write and read with identical needle points, jumps,
  trims and colour changes.
- `crates/engine/src/cmd/fileio/tests_stitchfile.rs`: the calibration design exported, opened and
  re-exported with the same needle points, `stitch.info` counts, the format table row, and the
  `bai_mirror` machine not warning for DSB.
- `crates/engine/tests/import_fuzz.rs`: `dsb_garbage_never_panics` and a mutated `.dsb` file.
- Cross-check against pyembroidery as a black box: see `docs/development.md`.

## Not done

- Whether a real machine accepts the header or needs particular fields: untested.
- Needle numbers are not read back into threads; pyembroidery and libembroidery do not map them
  either.
- Barudan-specific function codes beyond the table (speed, clutch) are not written or read.
