# Embroidery satin columns, second pass: corners, converted lettering, partial fills, underlay

> **Status:** implemented with STI-38, measured by tests and in StitchCraft (dev build); not stitched on a machine; connectors between letters revised with STI-41 (measured by tests and a decoded DST export) · **Date:** 2026-11-13 · **Covers:** gaps E9 and E10
> in [embroidery-parity.md](../../embroidery-parity.md) (Linear STI-38) · **Builds on:**
> [lettering](2026-11-12-embroidery-lettering.md), [auto-digitize](2026-11-13-embroidery-auto-digitize.md),
> [sequencing, second pass](2026-11-13-embroidery-sequencing-second-pass.md). Numbers marked
> *estimated* are starting points to check at the stitch-out.

## Goal

The owner sews lettering and small logos on a BAI Mirror. What shows first is the shape of satin
columns at corners and junctions, the trims, and how small letters look. This pass changes the
column code lettering and auto-digitize share (`columns.rs`) and what is built on it.

In scope, in this order: satin turning sharp corners as one column; converted lettering that
still sews as one lettering object; partial fills (tatami for the wide part of a glyph, satin for
the rest); underlay types; bottom join; smoother small running-stitch letters and an optional bean
run. Deferred: lap and cap-overlap corners, short-stitch settings of their own, double zigzag as
a separate choice, script fonts' connections, pre-digitized fonts.

## Sources consulted

Documentation only, read for behaviour; no code, constant or string was copied, so nothing is
added to `NOTICE`. No Adobe software or files; no GPL code (Ink/Stitch's documentation only).

| What | URL | Taken |
|---|---|---|
| Reference suite help, satin corners | <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Quality/quality/quality-9.htm>, <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Quality/quality/quality-11.htm> | Three corner styles: mitre (the edges meet in a point), cap (an extra segment caps a sharp corner), lap (two overlapping segments); sharper corners get caps |
| Ink/Stitch, satin column | <https://inkstitch.org/docs/stitches/satin-column/> | Rungs set the stitch direction through corners; centre-walk for thin columns, contour (edge) underlay inset from the edges, zig-zag plus contour for wide columns; short stitches in dense places |
| Reference suite help, underlay | <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Quality/stabilizing/stabilizing-10.htm>, <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Quality/stabilizing/stabilizing-12.htm> | Centre run for narrow columns, about 2–3 mm; edge run for larger shapes; zigzag for wide columns, alone or with an edge run |
| Sewing-software help, underlay | <https://softwarehelp.bernina.com/en/MainHelp/PART_Digitizing/quality/quality-5.htm>, <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Underlay_Stitches.htm> | The same 2–3 mm centre walk; edge walk and zigzag for large shapes; satin over 2 mm wants some underlay |
| Blog, underlay by width | <https://embroiderylegacy.com/?p=29571> | Centre run 1.5–2 mm, edge run 2.5–3.5 mm, edge run with zigzag from 4 mm |
| Reference suite help, lettering underlay | <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Lettering/lettering_advanced/lettering_advanced-22.htm> | By letter height: centre run at 6–10 mm, edge run above 10 mm, zigzag for jacket-back sizes |
| Reference suite help, join methods | <https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Lettering/lettering_advanced/lettering_advanced-16.htm> | Bottom join connects letters along the baseline (joins hide in towelling); closest join at the nearest points, recommended in most cases |
| Sewing-software help, joining points | <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Joining_Point.htm> | Baseline fonts start at a character's far left and end at its far right, near the baseline |
| Large lettering | <https://wilcom.com/resources/blog/create-large-lettering-for-garment-backs>, <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Large_Lettering.htm>, <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Overlaps_and_Underlay_Bars.htm> | Wide letter strokes become fill; adjacent blocks overlap slightly |
| Blog, small text | <https://digitizingbuddy.com/?p=5521> | A triple run gives very small letters thickness and visibility where satin collapses |

## 1. Corners

A degree-2 node of the skeleton graph that is left after pruning bends by 45° or more (a
straighter one is merged already). It is a **corner** when the two strokes meeting there are of
similar width (their median widths within a factor of two): the strokes are joined into one, and
the corner points are kept. A node where three or more strokes meet stays a junction (the T's
bar and stem, the middle of an E). A ring whose only node is a corner (a square O) is turned to
start in the middle of its longest side, so the closing point is not a corner.

Each stretch between corners (an **arm**) is smoothed and resampled on its own, so the corner
stays sharp, and gets its rails by normal casting as before. Near a corner, the rungs of each arm
are dropped within `h / tan(θ/2)` of the corner (`h` the half width there, `θ` the angle between
the arms), where a normal would reach across into the other arm. The gap is filled with corner
rungs:

- the **inner** rail folds: it runs from the first arm's last inner point to the inner corner
  (the outline hit along the inner bisector) and on to the second arm's first inner point;
- the **outer** rail goes round: to the mitre point where the arms' outer rails meet (held to
  the outline along the outer bisector), then on. When the mitre would reach more than twice the
  half width from the centre (a sharp corner), it is a **cap**: the outer rail is cut across
  square at twice the half width;
- the two halves are paired separately, so the rung from the inner corner to the mitre point is
  the diagonal of the corner.

The zigzag then fans round the inner corner. Satin needle points that would fall closer than a
quarter of the spacing to the one before on the same rail (the inner side of a corner, a tight
curve) are pulled a quarter of the rung in on every other stitch: short stitches, so the fabric
is not cut by one hole (*estimated* amounts).

Lettering and auto-digitize both take this (`columns::strokes`). Tests: an L, a V and a square
ring are one column each, cover the corner with no gap, and their rails never cross.

## 2. Converted lettering as one object

`text.toStitches` makes a group with stitch type **Lettering** holding a group per word, each
holding a group per letter, each holding the letter's pieces (two-rail satin columns, runs, a
fill), each piece in the direction and order lettering sews it. A group whose own stitch type is
Lettering is one sewn object (one entry in the sewing order, never turned round): its pieces are
gathered back into strokes (piece ends that meet are nodes) and sewn by the same router as live
type, with its walks, underlay and trims per word. A column keeps its own spacing,
compensation and underlay. A rail moved apart from its neighbours makes that column a separate
part of its letter: it is reached by a walk inside the letter or a trim, and only it sews
differently. Old converted groups (no Lettering type) sew as before.

Test: "Hello" converted sews 0 trims, as the live type does, and its stitch count is within 5 %.

## 3. Partial fills

A glyph whose strokes are wider than the widest satin was filled whole. Now its wide part (the
shape's opening by a disc as wide as the widest satin, as auto-digitize finds it) is tatami and
the rest stays columns and runs that reach 0.3 mm into the fill, all in the one letter, the fill
sewn first. The rule that a leftover shorter than half the widest satin is filled with the wide
part is auto-digitize's, shared. Auto-digitize already does this per shape; the split moves to a
function both use.

Test: a 25 mm block letter with 10 mm stem and 5 mm serifs: one tatami body, satin serifs.

## 4. Underlay types

`StitchProps.underlay` takes `auto` (default), `none`, `center`, `edge` or `zigzag`. Old files'
`true` reads as `auto` and `false` as `none`; `stitch.setParams` still takes `true` and `false`.
Automatic chooses by the column's median width as drawn: a centre run under 3 mm, an edge walk
from 3 mm, an edge walk and a zigzag from 6 mm (the sources give 2–3 mm for the centre run and
4 mm or "wide" for zigzag; 6 mm is *estimated*). Edge-walk rails are inset 20 % of the width,
0.2–0.5 mm (*estimated*); the zigzag is 2 mm peak to peak (*estimated*). Passes come in pairs
(out and back), so the satin starts where the underlay ends: centre out and back, edge on one
rail out and the other back, zigzag adds a zigzag out and back. Running-stitch letters have
none. The Stitch Properties panel shows a dropdown.

## 5. Bottom join

`StitchProps.letterJoin`: `closest` (default) or `bottom`. Bottom: each letter starts at its node
nearest its bottom-left corner and leaves towards the next letter's bottom-left, so letters are
joined along the baseline.

## 6. Small letters

Running-stitch letters (under `runUnder`, 5 mm) are smoothed more strongly (their skeletons are
a few pixels wide, and a staircase shows at 1.5 mm stitches), and the run stitch length is
shortened where the line bends, so a curve keeps within 0.1 mm of its centre line
(*estimated*). `StitchProps.bean` (lettering): a bean run (each stitch sewn three times) for
legibility, off by default.

## Connectors between letters

Review of the first screenshots: the walk from the T to the E of "TEXT" ran 8 mm down from the
T's bar to the E's middle bar, because joining the E's corner into one stroke took away the node
closest join had entered it by. Strokes are now split at their corners for routing only (the
column still turns the corner as one), so a letter is entered and left at a corner too; of the
points within 1 mm of the nearest, the one nearest the baseline is taken. A connector between
two letters of a word that would show more than 3 mm outside both letters' ink
(`MAX_CONNECTOR_MM`, *estimated*: the reference suite's help describes closest join as keeping
connectors short but gives no length) is a trim and a jump instead; letters never trimmed
(`letterTrims: none`) still walk. "Hello you" now trims three times (between the words, and
round the y's tail), where it trimmed once (once again since STI-41, below).

## Travel under the stitching (STI-41)

Decoding the DST of "I’m Conglei" at 40 pt (10 mm letters, word trims) showed 8 trims for 10
letters, from connectors of 4.4 to 8.9 mm: a letter ended wherever its last stroke ended, often
on the side away from the next letter. A ring (the o, the bowl of the e) ended where its single
node happened to sit on the outline; an open stroke (the C) at its far tip. The 3 mm rule was
right; the exits were wrong. Digitizers instead travel under the stitching to the side facing
the next letter (closest join keeps connectors short; Wilcom help, cited above). Now:

- A letter is left at its centre point nearest the next letter's centre lines (closest join;
  bottom join: nearest the next letter's bottom left). A letter with several parts (an i and its
  dot) leaves each part nearest the next part, and its last part nearest the next letter. The
  last letter of the text ends where its strokes do.
- It is entered at its centre point nearest the needle; walked to from the letter before, of the
  parts about as near (up to 8) the one the walk shows least of is entered (a j's stem, not its
  dot). A trimmed letter is jumped to straight at its exit.
- The stroke graph is cut at both points (a ring on its own is turned to start at the exit, so
  it stays one column). The shortest way between them along the centre lines is sewn last at
  each node; every other stroke is sewn out and back from the nodes on the way (centre run out
  under the satin, then the satin back), and a ring once round. So the part ends at the exit
  without a stitch across its counters.
- The 3 mm connector cap (`MAX_CONNECTOR_MM`), `NO_TRIM_GAP_MM`, `letterTrims` and the trim
  before each letter not walked to are unchanged; no bare jump is left (tested).

Measured with the plan's counters (Source Sans 3, as the plan tests set it; lead is the longest
walk between joined letters that shows outside their ink):

| Text, size, trims | Stitches before → after | Trims before → after | Longest lead, mm, before → after |
|---|---|---|---|
| "I’m Conglei", 40 pt, word | 1117 → 1134 | 8 → 2 | 2.45 → 2.38 |
| "Hello", 15 mm, word | 604 → 591 | 0 → 0 | 2.92 → 2.24 |
| "TEXT", 15 mm, word | 498 → 497 | 0 → 0 | 1.90 → 1.90 |
| "Hello you", 15 mm, word | 985 → 968 | 3 → 1 | 2.92 → 2.24 |
| "Hello you", 15 mm, none | 996 → 977 | 0 → 0 | 6.97 → 4.47 |
| "Hello you", 15 mm, letter | 975 → 948 | 7 → 7 | (none joined) |
| "ij", 15 mm, word | 152 → 143 | 2 → 2 | 2.19 → 2.75 |
| "Test", 5 mm, word | 41 → 44 | 0 → 0 | 1.16 → 0.89 |
| "Test", 12 mm, word | 368 → 367 | 1 → 0 | 1.33 → 2.87 |
| "on", "Co", "ol", 40 pt, word | 241, 243, 199 → 242, 235, 202 | 1 each → 0 | trimmed → 1.68, 2.24, 1.69 |

The two trims left in "I’m Conglei" are between the words and to the i's dot (1.4 mm above its
stem, past the 1.2 mm stitch gap). The exported DST (`vectorcraft-cli run`, dev build) decodes
to 1149 stitches and three jump groups: the first from the origin, then the two trims.

## Junctions

Looked at in StitchCraft: a 25 mm H's legs bulged into its bar (4.2 mm on a 3.2 mm leg), where a
normal at the junction reached across the bar. Near a junction with a through stroke, each
stroke's rails are now held to its median half width plus 15 % (*estimated*); the stroke meeting
it covers that side.

## Measured (StitchCraft dev build, DST exports decoded)

| Design | Stitches | Columns | Trims | Lone jumps after the first stitch |
|---|---|---|---|---|
| "TEXT", 15 mm type (cap 9.8 mm) | 502 | 13 | 0 | 0 |
| "TEXT", converted | 514 | 13 | 1 | 0 |
| "Hello", 15 mm, live | 606 | 10 | 0 | 0 |
| "Hello", converted | 622 | 10 | 0 | 0 |
| "Hello", bottom join | 590 | 10 | 0 | 0 |
| "H", 25 mm cap height | 417 | 5 | 0 | 0 |
| "Test", 5 mm type (cap 3.3 mm), running stitch | 43 | 0 | 0 | 0 |
| Serif I, 25 mm, auto-digitized | 552 | 4 + 1 fill | 4 | 0 |

Every DST starts with one jump from the origin to the first stitch, before any thread is sewn.
Converting a stroke into a column piece and a run piece at a junction travels a little more:
"TEXT" converted sews up to 10 % more than live (tested), "Hello" within 5 %.

## Deferred

- Small running-stitch letters are smoother but still angular on tight curves ("Test" at 5 mm,
  the s): their skeleton is a few pixels wide; a font-aware centre line would do better.
- The source fonts in the repository are regular weights, so no type at hand has strokes wider
  than satin: partial fills are tested on synthetic letters and on auto-digitize in the app.
- Lap corners, double zigzag as its own choice, underlay under fills and single-path satin.

## Compatibility

New fields are optional and read leniently (a value of another kind is unset). Files from this
version opened by older ones lose the new underlay kinds (read as unset: lettering's default),
`letterJoin` and `bean`.
