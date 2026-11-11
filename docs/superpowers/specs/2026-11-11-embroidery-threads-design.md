# Embroidery: thread charts, matching and needles

Status: first pass implemented 2026-11-11 (STI-12, gap E6). Measured by tests; no panel in the app.

The thread work of the embroidery subsystem: what a thread is, how a chart of them is imported, how
a design's colours are matched to a chart, and which needle sews which colour.

## Decision

The owner has not named a thread brand. No manufacturer publishes an openly licensed colour table,
and the tables bundled with GPL software may not be copied. So this pass ships **no brand chart**:
charts are **import-only**, plus a small original starter set. When a brand is named, a sample chart
for it is added to the tests and the user imports the real one.

No source from any other project was read for this: the thread record's fields (brand, catalogue
code, name, colour, weight) are those the issue lists.

## The thread record

`vectorcraft_stitch::ThreadSpec { brand, code, name, rgb, weight? }` is one chart entry.
`Thread` on a plan block (`rgb`, `name?`, `code?`, `brand?`) is what a block carries; a chart entry
turns into one with `ThreadSpec::thread()`, so formats and a later worksheet can print the brand and
code next to the colour. A `ThreadChart { name, threads }` is a list of specs.

## Chart files

Two formats, read by `ThreadChart::parse` (JSON when the first character is `{`, CSV otherwise):

- **CSV** (`.csv`): a header row, then one thread per row. Columns `code` and `hex` are required;
  `brand`, `name` and `weight` are optional; any order, any case. Fields holding a comma or a quote
  are quoted with `"` (`""` for a quote). Blank lines are skipped, a leading byte order mark is
  ignored. `hex` is `#rrggbb` or `rrggbb`. A missing `brand` is the chart's name. The chart is named
  after the file.

  ```
  brand,code,name,hex,weight
  Acme,100,Cherry,#c81e28,40wt
  Acme,200,"Ocean, deep",#1e50c8,
  ```

- **JSON** (`.json`): `{"format": "vectorcraft-threads", "name": "Acme 40", "threads": [{"brand",
  "code", "name", "hex", "weight"}, ...]}`. `format` and `name` may be left out; `brand`, `name` and
  `weight` are optional per thread.

Caps (the file is hostile input): 4 MB, 20,000 threads, brand 64, code 32, name 96, weight 16
characters, chart name 96; no control characters; UTF-8 only. A code listed twice for one brand, an
empty code, a bad colour, a quote inside an unquoted field or an unclosed quote is an error naming
the line (CSV) or thread number (JSON). Nothing is truncated silently. A path past the size cap is
refused before it is read in full.

Charts are kept like the swatch and style libraries (`Libraries<ThreadChart>`): the desktop app sets
the user folder to `Threads` next to the preferences, its `.csv` and `.json` files are listed as
`user/<file>` and rescanned on `stitch.threads.list`; an import without `user` is listed as
`loaded/<name>` until the app quits; `user: true` also writes it to the folder (CSV files as they
are, other data as JSON). Without a user folder (the web, headless runs) only loaded charts exist.

## The starter set

`starter_chart()`: 24 colours generated in code, brand `VectorCraft`, codes `VC01`-`VC24`: black,
charcoal, grey, silver, pale grey, white, twelve hues spread round the colour circle, and six mixed
shades (brown, navy, forest, burgundy, pink, sky). It is a starter set so a design can be named and
numbered at once. It is not a brand chart and matches no maker's range. Every pair is more than 5
delta E apart (tested).

## Matching

`ThreadChart::nearest(rgb)` picks the chart thread with the smallest **CIE76 delta E** (Euclidean
distance in CIE L\*a\*b\*, D65) to a colour; the first of equal distances wins; an empty chart has no
match. CIE76 is cheap and good enough to pick among a few hundred threads. CIEDE2000 separates near
neighbours better and may replace it; the match result carries the distance so callers can show it.
`ThreadChart::match_plan(plan)` gives one match per block.

## Needles

`assign_needles(plan, needles)` gives each distinct colour, in the order it is first sewn, the next
needle from 1 up to the machine's count (`needles` of the profile: 15 for `commercial15` and
`bai_mirror`, none for `generic`); a colour sewn again keeps its needle. Colours past the count get
no needle (`null`) and one warning saying how many need a thread swap during the job.

The assignment is informational. The DSB writer still changes needles 1, 2, 3 ... per colour change
in sewing order (wrapping after 15) and does not read it; making the writer follow the assignment
needs the needle to travel in `WriteOptions` or on the plan, and is left for the pass that puts needle
numbers on the worksheet (E7).

## Commands

| Command | Does |
|---|---|
| `stitch.threads.list {chart?}` | The starter set, the Threads folder's charts and loaded ones, which is active; with `chart`, its threads. Query. |
| `stitch.threads.import {path? \| data? \| dataBase64?, name?, user?}` | Reads a chart; errors name the line. |
| `stitch.threads.use {name?}` | Makes a chart the active one (id or name, any case); `none` or nothing clears it. Kept for the session; storing it in the preferences is left for the Threads panel. |
| `stitch.threads.match {chart?, ...plan options}` | The nearest thread of the chart (default the active one) to each block's colour: `{block, rgb, brand, code, name, thread, deltaE}`. Query. |
| `stitch.needles {machine?, chart?, ...plan options}` | `{machine, needleCount, needles: [{needle, rgb, stitches, blocks, chart?}], warnings}`. Query. |

With a chart active, `stitch.info`'s `threads` rows gain `brand`, `code`, `name` and `deltaE`.
`stitch.threads.use` and `.import` are not callable from plug-ins (they change app state); the
queries are.

## Tests

- `vectorcraft-stitch` (`thread/tests.rs`): CSV quirks (quotes, blank lines, bare hex, byte order mark,
  column order), JSON and CSV agreement and round trip, each error with its line, the caps, the
  starter set (24, distinct, original names), Lab against known values, nearest and ties, per-block
  matching, needle order, reuse and the over-count warning, property tests for garbage and for
  nearest being never worse than any other thread.
- `vectorcraft-engine` (`tests/stitchthreads.rs`): list, import from data and from a path, the
  Threads folder and a second session reading it, the size cap, `use`, `stitch.info` names, `match`,
  `needles` with 3, 15 and 16 colours.
- `engine/tests/import_fuzz.rs` (`thread_*`): garbage, chart-shaped text and mutated CSV and JSON
  charts, read directly and imported, then used with `use`, `stitch.info`, `match` and `needles`.
- `command_sweep` and the plug-in review list cover the new command ids.

## Deferred

Colourways (several colour schemes per design); a Threads panel and the Embroidery menu entries; the
active chart in the preferences; brand charts (import-only until a brand is named, then a sample
chart in the tests); the worksheet (E7) with thread codes and needle numbers; the DSB writer
following the assignment; CIEDE2000; thread length per colour from the chart's weight; editing a
chart in the app.
