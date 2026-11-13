//! Lettering: type sewn as satin columns along each letter's strokes (E9).
//!
//! A glyph outline (millimetres, holes included, nonzero rule) is reduced to its skeleton
//! ([`skeleton`]): a graph of strokes whose centre lines run down the middle of the letter. Each
//! stroke gets two rails, found by casting its normals both ways to the outline, so the column
//! hugs the letter as it thickens and thins; the column is sewn by [`crate::rails_satin`]. A stroke
//! narrower than the minimum column is sewn as running stitch, a letter whose strokes are wider
//! than satin takes as tatami, and letters shorter than the run height entirely as running stitch
//! along the skeleton.
//!
//! The thresholds come from public digitizing guides (see
//! `docs/superpowers/specs/2026-11-12-embroidery-lettering.md`):
//! - letters under about 5 mm are sewn as running stitch: mixed case is not sewn as satin under
//!   5 mm (<https://www.piworld.com/embroidery-digitizing-tips-on-creating-designs-with-letters/>,
//!   <https://blog.ignitiondrawing.com/small-lettering-guide-part-1/>);
//! - small satin letters take a centre-run underlay, never an edge walk
//!   (<https://members.asicentral.com/news/how-to/may-2018/keep-your-small-letters-crisp/>);
//! - a column is not narrower than about 1 mm, the needle (ASI, above;
//!   <https://graphics-pro.com/?p=95539>);
//! - satin wider than about 8 mm becomes fill (Graphics Pro, above;
//!   <https://softwarehelp.mysewnet.com/MSW/120/FontDigitizing/Topics/Large_Lettering.htm>);
//! - letters closer than 1.2 mm need no trim (piworld, above).
//!
//! Inside a letter the strokes are sewn in one go, travelling under the satin that covers them
//! later. A letter is entered at its point nearest the needle and left at its point nearest the
//! next letter (a ring is cut there, so an o starts and ends on the side facing the next
//! letter): the strokes on the way between them are sewn last, everything else out and back,
//! so the needle travels under the stitching rather than across the gap. Letters are trimmed
//! per word by default, and wherever the walk between two would show over
//! [`MAX_CONNECTOR_MM`].

#[cfg(test)]
mod tests;

use vectorcraft_doc::{LetterJoin, LetterTrims, Underlay};
use vectorcraft_geom::FillRule;

use crate::autodigitize::{Split, split_wide};
use crate::columns::{self, Stroke, dist, extend_nodes, finite, pieces};
use crate::fill::{FillParams, tatami};
use crate::run::run_stitches;
use crate::satin::{Pass, RailsParams, pass_stitches, passes, rails_satin, underlay_for};
use crate::{MAX_STEPS, Pt, TOO_MANY_STEPS, UNITS_PER_MM};

pub(crate) type P = (f64, f64);

/// The narrowest satin column, mm: narrower strokes are sewn as running stitch.
pub const MIN_COLUMN_MM: f64 = 1.0;
/// Letters shorter than this, mm, are sewn as running stitch.
pub const RUN_UNDER_MM: f64 = 5.0;
/// The widest satin column, mm: a letter with wider strokes is sewn as tatami.
pub const MAX_SATIN_MM: f64 = 8.0;
/// Pull compensation for lettering, mm each side (estimated: no public source gives one value).
pub const COMPENSATION_MM: f64 = 0.2;
/// Running stitch length of lettering runs, mm (estimated).
pub const RUN_LENGTH_MM: f64 = 1.5;
/// Stitch length of the centre-run underlay and of travel under satin, mm.
pub const UNDERLAY_LENGTH_MM: f64 = 2.0;
/// How far a running-stitch letter's stitch may stray from its centre line, mm: stitches are
/// shortened on curves to keep within it (estimated).
pub const RUN_CHORD_MM: f64 = 0.1;
/// The shortest running stitch a curve is cut into, mm (estimated).
pub const RUN_MIN_MM: f64 = 0.4;
/// Smoothing passes over a running-stitch letter's centre lines, beyond the columns' own.
const RUN_SMOOTHING: usize = 6;
/// A running-stitch letter's corner stays one where its line turns by more than this, measured
/// [`RUN_CORNER_REACH_MM`] each side (a thin stroke's skeleton marks many small bends as corners).
const RUN_CORNER_TURN_DEG: f64 = 75.0;
/// How far each side of a corner its turn is measured, mm.
const RUN_CORNER_REACH_MM: f64 = 0.3;
/// The longest connector between two letters of a word that may show on the fabric, mm: a walk
/// from one letter to the next with more than this outside both letters' ink is a trim and a jump
/// instead
/// (unless the letters are never trimmed, [`LetterTrims::None`]). Closest join exists to keep
/// these connectors short (<https://docs.wilcom.com/embroiderystudio/28/en/OnlineHelp/Lettering/lettering_advanced/lettering_advanced-16.htm>);
/// no public source gives a length, so 3 mm is estimated.
pub const MAX_CONNECTOR_MM: f64 = 3.0;
/// Letters closer than this, mm, are joined by a stitch rather than a jump.
pub const NO_TRIM_GAP_MM: f64 = 1.2;
/// How far apart the points are that check whether a walk stays inside a letter, mm.
const HIDDEN_STEP_MM: f64 = 0.25;
/// The most points one such check samples.
const HIDDEN_SAMPLES: usize = 400;
/// The most letters one text object sews.
pub const MAX_LETTERS: usize = 10_000;

/// How lettering is sewn. Lengths in millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LetteringParams {
    /// Satin spacing, peak to peak.
    pub spacing: f64,
    /// Running stitch length of small letters and thin strokes.
    pub run_length: f64,
    /// Pull compensation, each side.
    pub compensation: f64,
    /// The narrowest satin column.
    pub min_column: f64,
    /// Letters shorter than this are running stitch.
    pub run_under: f64,
    /// The widest satin column.
    pub max_satin: f64,
    /// The underlay under satin columns (automatic: by each column's width).
    pub underlay: Underlay,
    pub trims: LetterTrims,
    /// Where one letter is joined to the next: closest, or along the baseline.
    pub join: LetterJoin,
    /// Running-stitch letters sewn as a bean run (each stitch three times).
    pub bean: bool,
    /// The longest stitch.
    pub max_stitch: f64,
    /// Tatami settings for letters too wide for satin.
    pub fill: FillParams,
}

impl Default for LetteringParams {
    fn default() -> Self {
        Self {
            spacing: 0.4,
            run_length: RUN_LENGTH_MM,
            compensation: COMPENSATION_MM,
            min_column: MIN_COLUMN_MM,
            run_under: RUN_UNDER_MM,
            max_satin: MAX_SATIN_MM,
            underlay: Underlay::Auto,
            trims: LetterTrims::Word,
            join: LetterJoin::Closest,
            bean: false,
            max_stitch: 12.1,
            fill: FillParams::default(),
        }
    }
}

/// How a letter is sewn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LetterMode {
    /// Satin columns (thin strokes as running stitch).
    Satin,
    /// Running stitch along the strokes.
    Run,
    /// Tatami fill of the whole letter.
    Fill,
}

impl LetterMode {
    pub fn id(self) -> &'static str {
        match self {
            LetterMode::Satin => "satin",
            LetterMode::Run => "run",
            LetterMode::Fill => "fill",
        }
    }
}

/// One letter to sew: its outline in mm (closed rings), thread and word.
#[derive(Clone, Debug, PartialEq)]
pub struct Letter {
    pub rings: Vec<Vec<P>>,
    pub rgb: [u8; 3],
    /// Which word (or line) of the text it belongs to: letters of one word are not trimmed apart
    /// by [`LetterTrims::Word`].
    pub word: usize,
}

/// A satin column: two rails, paired point by point.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub left: Vec<P>,
    pub right: Vec<P>,
}

impl Column {
    /// The narrowest rung as drawn, mm; it sews this plus twice the pull compensation wide, which is
    /// at least the minimum column.
    pub fn min_width(&self) -> f64 {
        self.left.iter().zip(&self.right).map(|(a, b)| dist(*a, *b)).fold(f64::INFINITY, f64::min)
    }
}

/// What a letter becomes, as editable objects ([`shape_letter`]).
#[derive(Clone, Debug, PartialEq)]
pub struct LetterShape {
    pub mode: LetterMode,
    pub columns: Vec<Column>,
    /// Running stitch lines (thin strokes, or the whole letter when small).
    pub runs: Vec<Vec<P>>,
    /// The outline filled: the whole letter (fill mode), or its wide part, sewn before the columns.
    pub fill: Vec<Vec<P>>,
}

/// A sewn letter.
#[derive(Clone, Debug, PartialEq)]
pub struct SewnLetter {
    pub rgb: [u8; 3],
    pub word: usize,
    pub mode: LetterMode,
    /// Needle points. The first run follows the letter before it: a trim and a jump, unless
    /// `joined`. The other runs are separate parts of the letter (an i's dot) that a trim and a
    /// jump reach; a part the needle can walk to inside the letter's ink, or within
    /// [`NO_TRIM_GAP_MM`], is in the run before it instead, with the walk's stitches.
    pub runs: Vec<Vec<Pt>>,
    /// The first run continues from where the last letter ended, its stitches to the letter
    /// included: no trim or jump before it. Letters of one word are joined this way by
    /// [`LetterTrims::Word`], every letter by [`LetterTrims::None`].
    pub joined: bool,
    pub columns: usize,
    pub run_lines: usize,
    /// Fills sewn (a filled letter, or the wide parts of one).
    pub fills: usize,
    /// When joined, how much of the walk from the letter before shows outside their ink, mm
    /// (0 otherwise).
    pub lead_mm: f64,
}

/// The letters' height: the tallest outline, mm.
pub fn letter_height(letters: &[Letter]) -> f64 {
    letters
        .iter()
        .map(|l| {
            let ys = l.rings.iter().flatten().map(|p| p.1).filter(|y| y.is_finite());
            let (lo, hi) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| (lo.min(y), hi.max(y)));
            if hi >= lo { hi - lo } else { 0.0 }
        })
        .fold(0.0, f64::max)
}

/// The mode letters of height `height` take: running stitch under [`LetteringParams::run_under`],
/// else satin.
pub fn mode_for(height: f64, p: &LetteringParams) -> LetterMode {
    if height < p.run_under { LetterMode::Run } else { LetterMode::Satin }
}

/// A letter analysed: its fills (sewn first) and its strokes and nodes, or its mode when it has
/// no strokes to sew.
#[derive(Clone)]
struct Analysis {
    mode: LetterMode,
    nodes: Vec<P>,
    strokes: Vec<Stroke>,
    /// Each stroke's own satin settings (a converted letter's column), else the lettering's.
    style: Vec<Option<PieceStyle>>,
    /// Outlines (rings, nonzero rule) sewn as tatami before the strokes.
    fills: Vec<Vec<Vec<P>>>,
    /// Each stroke that goes on from the one before it round a corner ([`split_corners`]): its
    /// column is the same column, not counted again. Empty: none.
    continues: Vec<bool>,
}

/// How one column of a converted letter sews, from its own stitch properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceStyle {
    /// Satin spacing, peak to peak, mm.
    pub spacing: f64,
    /// Pull compensation each side, mm.
    pub compensation: f64,
    /// The underlay under it.
    pub underlay: Underlay,
}

/// A converted letter (`text.toStitches`): its pieces as stitch objects hold them, to be sewn as
/// lettering again ([`sew_pieces`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PieceLetter {
    pub rgb: [u8; 3],
    pub word: usize,
    /// Two-rail satin columns, each with its own settings.
    pub columns: Vec<(Column, PieceStyle)>,
    /// Running stitch lines.
    pub runs: Vec<Vec<P>>,
    /// Fill outlines (rings), each one fill.
    pub fills: Vec<Vec<Vec<P>>>,
}

/// The analysis of a glyph outline in `mode` (satin or run). Hostile outlines are bounded: more
/// than [`crate::skeleton::MAX_SEGMENTS`] segments is an error, more strokes than
/// [`columns::MAX_EDGES`] sews the letter as fill.
fn analyse(rings: &[Vec<P>], mode: LetterMode, p: &LetteringParams) -> Result<Analysis, String> {
    columns::check_outline(rings, "letter")?;
    let empty = |mode| {
        let fills = if mode == LetterMode::Fill { vec![rings.to_vec()] } else { vec![] };
        Analysis { mode, nodes: vec![], strokes: vec![], style: vec![], fills, continues: vec![] }
    };
    if mode == LetterMode::Fill {
        return Ok(empty(LetterMode::Fill));
    }
    let Some(sk) = columns::skeleton_of(rings, "letter")? else { return Ok(empty(mode)) };
    let Some(gr) = sk.graph else { return Ok(empty(LetterMode::Fill)) };
    // The sewn width, compensation included, is what the minimum holds.
    let comp = 2.0 * p.compensation.max(0.0);
    let strokes = columns::strokes(&gr, &sk.grid, &sk.segs, |w| mode == LetterMode::Satin && w + comp >= p.min_column);
    let nodes: Vec<P> = gr.nodes.iter().map(|n| n.pos).collect();
    // A letter with strokes too wide for satin is filled.
    if mode == LetterMode::Satin {
        let widest = strokes
            .iter()
            .map(|s| {
                let mut w: Vec<f64> = s.left.iter().zip(&s.right).map(|(a, b)| dist(*a, *b)).collect();
                w.sort_by(f64::total_cmp);
                w.get(w.len() * 4 / 5).copied().unwrap_or(0.0)
            })
            .fold(0.0, f64::max);
        if widest > p.max_satin {
            // Its wide part is tatami, the rest stays columns and runs reaching into it; when
            // nothing is left over, all of it is filled.
            return Ok(match split_wide(&sk.grid, &strokes, p.max_satin) {
                Split::Mixed { fill, stretches } => {
                    let style = vec![None; stretches.len()];
                    Analysis { mode, nodes: extend_nodes(nodes, &stretches), strokes: stretches, style, fills: vec![fill], continues: vec![] }
                }
                Split::Narrow | Split::Whole => empty(LetterMode::Fill),
            });
        }
    }
    let mut strokes = strokes;
    if mode == LetterMode::Run {
        for s in &mut strokes {
            smooth_run(s);
        }
    }
    let style = vec![None; strokes.len()];
    Ok(Analysis { mode, nodes: extend_nodes(nodes, &strokes), strokes, style, fills: vec![], continues: vec![] })
}

/// Piece ends of a converted letter are one node when closer than this many half widths of the
/// column ending there (a column's end is the middle of its last rung, which lies off the centre
/// line where a rail reaches into another stroke).
const SAME_NODE: f64 = 0.8;
/// ... and always when closer than this, mm.
const SAME_POINT_MM: f64 = 0.05;

/// Whether each point of `pts` is a corner: the line turns there by more than 40 degrees,
/// measured two points each side, and more than at its neighbours.
fn bends(pts: &[P]) -> Vec<bool> {
    let turn = |i: usize| -> f64 {
        let (Some(a), Some(b), Some(c)) = (i.checked_sub(2).and_then(|j| pts.get(j)), pts.get(i), pts.get(i + 2)) else { return 0.0 };
        let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
        let (lu, lv) = (u.0.hypot(u.1), v.0.hypot(v.1));
        if lu > 1e-9 && lv > 1e-9 { ((u.0 * v.0 + u.1 * v.1) / (lu * lv)).clamp(-1.0, 1.0).acos().to_degrees() } else { 0.0 }
    };
    let turns: Vec<f64> = (0..pts.len()).map(turn).collect();
    (0..pts.len())
        .map(|i| {
            let t = turns.get(i).copied().unwrap_or(0.0);
            t > 40.0 && (i.saturating_sub(2)..=(i + 2)).all(|j| j == i || turns.get(j).is_none_or(|u| *u < t || (*u == t && j > i)))
        })
        .collect()
}

/// The polygon of a column's rails, counter-clockwise (positive area).
fn column_ring(c: &Column) -> Vec<P> {
    let mut ring = c.left.clone();
    ring.extend(c.right.iter().rev());
    orient(ring)
}

/// `ring` turned to positive area.
fn orient(mut ring: Vec<P>) -> Vec<P> {
    let n = ring.len();
    let area: f64 = (0..n).filter_map(|i| Some((ring.get(i)?, ring.get((i + 1) % n)?))).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum();
    if area < 0.0 {
        ring.reverse();
    }
    ring
}

/// A converted letter gathered back into strokes: piece ends that meet are nodes; its ink, for
/// walks hidden under it, is its columns' outlines and its fills.
fn from_pieces(l: &PieceLetter) -> Result<Prepared, String> {
    // Each node: where it is and the half width of the column end that made it.
    let mut nodes: Vec<(P, f64)> = vec![];
    // The node at `q` (half width `r`), never `not` (a stroke's other end, unless it closes).
    let mut node = |q: P, r: f64, not: Option<usize>| -> usize {
        let reach = |n: &(P, f64)| (SAME_NODE * r.max(n.1)).max(SAME_POINT_MM);
        let near = nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| Some(*i) != not && dist(n.0, q) <= reach(n))
            .min_by(|x, y| dist(x.1.0, q).total_cmp(&dist(y.1.0, q)))
            .map(|(i, _)| i);
        near.unwrap_or_else(|| {
            nodes.push((q, r));
            nodes.len() - 1
        })
    };
    let half = |c: &Column, i: usize| match (c.left.get(i), c.right.get(i)) {
        (Some(a), Some(b)) => dist(*a, *b) / 2.0,
        _ => 0.0,
    };
    let mut strokes = vec![];
    let mut style = vec![];
    for (c, st) in &l.columns {
        let centre: Vec<P> = c.left.iter().zip(&c.right).map(|(a, b)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)).collect();
        if centre.len() < 2 || c.right.len() != centre.len() || !centre.iter().all(|q| finite(*q)) {
            continue;
        }
        let (Some(&first), Some(&last)) = (centre.first(), centre.last()) else { continue };
        let n = centre.len();
        let a = node(first, half(c, 0), None);
        let b = node(last, half(c, n - 1), (dist(first, last) > SAME_POINT_MM).then_some(a));
        strokes.push(Stroke { a, b, bend: bends(&centre), centre, left: c.left.clone(), right: c.right.clone(), column: vec![true; n] });
        style.push(Some(*st));
    }
    for line in &l.runs {
        let (Some(&first), Some(&last)) = (line.first(), line.last()) else { continue };
        if line.len() < 2 || !line.iter().all(|q| finite(*q)) {
            continue;
        }
        let a = node(first, 0.0, None);
        let b = node(last, 0.0, (dist(first, last) > SAME_POINT_MM).then_some(a));
        let n = line.len();
        strokes.push(Stroke { a, b, bend: bends(line), centre: line.clone(), left: line.clone(), right: line.clone(), column: vec![false; n] });
        style.push(None);
    }
    let mut rings: Vec<Vec<P>> = l.columns.iter().map(|(c, _)| column_ring(c)).collect();
    for f in &l.fills {
        columns::check_outline(f, "letter")?;
        // A fill's outer ring counter-clockwise, its holes the other way, as glyphs wind.
        let flip = f.first().is_some_and(|r| orient(r.clone()) != *r);
        rings.extend(f.iter().map(|r| if flip { r.iter().rev().copied().collect() } else { r.clone() }));
    }
    let mode = if !l.columns.is_empty() {
        LetterMode::Satin
    } else if !l.runs.is_empty() {
        LetterMode::Run
    } else {
        LetterMode::Fill
    };
    let nodes = nodes.into_iter().map(|n| n.0).collect();
    let mut a = Analysis { mode, nodes: extend_nodes(nodes, &strokes), strokes, style, fills: l.fills.clone(), continues: vec![] };
    split_corners(&mut a);
    Ok(Prepared { rings, rgb: l.rgb, word: l.word, a })
}

/// Split each stroke of `a` at its corners, for routing only: a letter may be entered and left
/// at a corner (the top left of an E), as at the ends of its strokes. The pieces share the
/// corner's rung, so the column still turns the corner as one.
fn split_corners(a: &mut Analysis) {
    let mut strokes = Vec::with_capacity(a.strokes.len());
    let mut style = Vec::with_capacity(a.strokes.len());
    let mut continues = Vec::with_capacity(a.strokes.len());
    for (i, s) in std::mem::take(&mut a.strokes).into_iter().enumerate() {
        let own = a.style.get(i).copied().flatten();
        let n = s.centre.len();
        // Corners at least three points from each end and from each other.
        let mut cuts: Vec<usize> = vec![];
        for k in 3..n.saturating_sub(3) {
            if s.bend.get(k).copied().unwrap_or(false) && cuts.last().is_none_or(|c| k >= c + 3) {
                cuts.push(k);
            }
        }
        if cuts.is_empty() || s.a == s.b && cuts.len() < 2 {
            strokes.push(s);
            style.push(own);
            continues.push(false);
            continue;
        }
        let mut from = 0;
        let mut node = s.a;
        let ends: Vec<usize> = cuts.iter().copied().chain([n - 1]).collect();
        for (j, &to) in ends.iter().enumerate() {
            let next = if j + 1 == ends.len() {
                s.b
            } else {
                a.nodes.push(s.centre.get(to).copied().unwrap_or((0.0, 0.0)));
                a.nodes.len() - 1
            };
            let part = |v: &Vec<P>| v.get(from..=to).map(<[P]>::to_vec).unwrap_or_default();
            let mut bend = s.bend.get(from..=to).map(<[bool]>::to_vec).unwrap_or_default();
            if let Some(b) = bend.first_mut() {
                *b = false;
            }
            if let Some(b) = bend.last_mut() {
                *b = false;
            }
            strokes.push(Stroke {
                a: node,
                b: next,
                centre: part(&s.centre),
                left: part(&s.left),
                right: part(&s.right),
                column: s.column.get(from..=to).map(<[bool]>::to_vec).unwrap_or_default(),
                bend,
            });
            style.push(own);
            continues.push(j > 0);
            from = to;
            node = next;
        }
    }
    a.strokes = strokes;
    a.style = style;
    a.continues = continues;
}

/// A letter ready to sew: its ink (rings, nonzero rule), thread, word and analysis.
struct Prepared {
    rings: Vec<Vec<P>>,
    rgb: [u8; 3],
    word: usize,
    a: Analysis,
}

/// What letter `rings` becomes in `mode` (satin or run; a letter too wide for satin becomes
/// fill), as columns, run lines and fill outlines: the objects Convert to Stitch Objects makes.
pub fn shape_letter(rings: &[Vec<P>], mode: LetterMode, p: &LetteringParams) -> Result<LetterShape, String> {
    let a = analyse(rings, mode, p)?;
    let mut shape = LetterShape { mode: a.mode, columns: vec![], runs: vec![], fill: a.fills.first().cloned().unwrap_or_default() };
    for s in &a.strokes {
        for (col, from, to) in pieces(s) {
            if col {
                let (Some(l), Some(r)) = (s.left.get(from..=to), s.right.get(from..=to)) else { continue };
                shape.columns.push(Column { left: l.to_vec(), right: r.to_vec() });
            } else if let Some(c) = s.centre.get(from..=to) {
                shape.runs.push(c.to_vec());
            }
        }
    }
    Ok(shape)
}

/// Needle points, kept in runs: a new run where the needle must jump.
struct Out {
    runs: Vec<Vec<Pt>>,
    budget: usize,
    /// The longest stitch, mm.
    max: f64,
    count: usize,
}

impl Out {
    fn pos(&self) -> Option<Pt> {
        self.runs.last().and_then(|r| r.last()).copied()
    }

    /// Sew `pts` on from where the needle is; the stitch to their first point is split when longer
    /// than the maximum.
    fn add(&mut self, pts: &[Pt]) -> Result<(), String> {
        let max = self.max;
        let Some(run) = self.runs.last_mut() else { return Ok(()) };
        if let (Some(&a), Some(&b)) = (run.last(), pts.first()) {
            let d = a.dist(b) / UNITS_PER_MM;
            if d > max {
                let k = (d / max).ceil().min(1000.0) as usize;
                let (a, b) = (a.to_mm(), b.to_mm());
                for j in 1..k {
                    let t = j as f64 / k as f64;
                    if let Some(q) = Pt::from_mm(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t) {
                        run.push(q);
                        self.count += 1;
                    }
                }
            }
        }
        for &q in pts {
            if run.last() != Some(&q) {
                run.push(q);
                self.count += 1;
            }
        }
        if self.count > self.budget {
            return Err(TOO_MANY_STEPS.into());
        }
        Ok(())
    }
}

/// Plan units of a length in mm, at least one.
fn units(mm: f64) -> i32 {
    // Lengths are checked to their ranges (at most 20 mm).
    ((mm * UNITS_PER_MM).round() as i32).max(1)
}

/// The depth-first walk over a letter's strokes.
struct Router<'a> {
    /// The letter's analysis, its strokes cut where a part is entered and left ([`Router::cut`]).
    a: Analysis,
    p: &'a LetteringParams,
    sewn: Vec<bool>,
    seen: Vec<bool>,
    /// The strokes of the way from where a part is entered to where it is left: each is sewn last
    /// at the node it starts from, after everything else there, so the part ends where the way
    /// does.
    way: Vec<bool>,
    /// Whether the part may end anywhere (the last part of the last letter): its last stroke at
    /// each node is sewn on, not out and back.
    free: bool,
    target: P,
    out: Out,
    columns: usize,
    run_lines: usize,
}

/// Cuts closer than this many centre points to a stroke's end are made at the end.
const CUT_SNAP: usize = 2;

impl Router<'_> {
    /// The strokes at node `u` not yet sewn, but for those of the way.
    fn open_at(&self, u: usize) -> Vec<usize> {
        self.a
            .strokes
            .iter()
            .enumerate()
            .filter(|(i, s)| (s.a == u || s.b == u) && !self.sewn.get(*i).copied().unwrap_or(true) && !self.way.get(*i).copied().unwrap_or(false))
            .map(|(i, _)| i)
            .collect()
    }

    /// The stroke of the way at node `u` not yet sewn.
    fn way_at(&self, u: usize) -> Option<usize> {
        self.a
            .strokes
            .iter()
            .enumerate()
            .position(|(i, s)| (s.a == u || s.b == u) && self.way.get(i).copied().unwrap_or(false) && !self.sewn.get(i).copied().unwrap_or(true))
    }

    /// Whether stroke `i` is still to sew.
    fn unsewn(&self, i: usize) -> bool {
        !self.sewn.get(i).copied().unwrap_or(true)
    }

    /// The strokes not yet sewn, grouped into parts: strokes that meet at a node are one part.
    /// Each stroke's part, `None` once sewn.
    fn parts(&self) -> Vec<Option<usize>> {
        // Union-find over the nodes, bounded by their number.
        let mut up: Vec<usize> = (0..self.a.nodes.len()).collect();
        fn root(up: &mut [usize], mut x: usize) -> usize {
            for _ in 0..up.len() {
                let Some(&p) = up.get(x) else { break };
                if p == x {
                    break;
                }
                let gp = up.get(p).copied().unwrap_or(p);
                if let Some(u) = up.get_mut(x) {
                    *u = gp;
                }
                x = p;
            }
            x
        }
        for (i, s) in self.a.strokes.iter().enumerate() {
            if self.unsewn(i) {
                let (ra, rb) = (root(&mut up, s.a), root(&mut up, s.b));
                if let Some(u) = up.get_mut(ra) {
                    *u = rb;
                }
            }
        }
        (0..self.a.strokes.len()).map(|i| if self.unsewn(i) { self.a.strokes.get(i).map(|s| root(&mut up, s.a)) } else { None }).collect()
    }

    /// The centre point of the strokes of part `part` (any part: `None`) nearest by `key`: its
    /// stroke and index.
    fn nearest(&self, parts: &[Option<usize>], part: Option<usize>, key: impl Fn(P) -> f64) -> Option<(usize, usize)> {
        let mut best: Option<(f64, usize, usize)> = None;
        for (i, s) in self.a.strokes.iter().enumerate() {
            let Some(Some(pi)) = parts.get(i) else { continue };
            if part.is_some_and(|x| x != *pi) {
                continue;
            }
            for (k, q) in s.centre.iter().enumerate() {
                let d = key(*q);
                if d.is_finite() && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i, k));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// The node at centre point `k` of stroke `i`: a stroke end when within [`CUT_SNAP`] of it,
    /// else the stroke is cut there in two. A closed ring on its own (an o) is turned to start
    /// there instead when `turn`, so it stays one column that starts and ends at the point.
    fn cut(&mut self, i: usize, k: usize, turn: bool) -> Option<usize> {
        let s = self.a.strokes.get(i)?;
        let n = s.centre.len();
        let alone = s.a == s.b && self.a.strokes.iter().enumerate().all(|(j, t)| j == i || (t.a != s.a && t.b != s.a));
        if turn && alone && n >= 4 && k > 0 && k + 1 < n {
            let s = self.a.strokes.get_mut(i)?;
            // Closed: the last point repeats the first, or the ring closes across from it.
            let closed = s.centre.first().zip(s.centre.last()).is_some_and(|(f, l)| dist(*f, *l) < 1e-9);
            let cyc = if closed { n - 1 } else { n };
            let order: Vec<usize> = (k..cyc).chain(0..=k).collect();
            let turned = |v: &Vec<P>| order.iter().filter_map(|&j| v.get(j).copied()).collect::<Vec<P>>();
            let (centre, left, right) = (turned(&s.centre), turned(&s.left), turned(&s.right));
            let column: Vec<bool> = order.iter().map(|&j| s.column.get(j).copied().unwrap_or(false)).collect();
            let mut bend: Vec<bool> = order.iter().map(|&j| s.bend.get(j).copied().unwrap_or(false)).collect();
            if let Some(b) = bend.first_mut() {
                *b = false;
            }
            if let Some(b) = bend.last_mut() {
                *b = false;
            }
            if centre.len() != order.len() || left.len() != order.len() || right.len() != order.len() {
                return Some(s.a);
            }
            let q = centre.first().copied();
            (s.centre, s.left, s.right, s.column, s.bend) = (centre, left, right, column, bend);
            let node = s.a;
            if let (Some(q), Some(pos)) = (q, self.a.nodes.get_mut(node)) {
                *pos = q;
            }
            return Some(node);
        }
        let s = self.a.strokes.get(i)?;
        if k < CUT_SNAP || k + CUT_SNAP >= n {
            return Some(if 2 * k < n { s.a } else { s.b });
        }
        let node = self.a.nodes.len();
        let mut bend: Vec<bool> = s.bend.clone();
        if let Some(b) = bend.get_mut(k) {
            *b = false;
        }
        let head = |v: &Vec<P>| v.get(..=k).map(<[P]>::to_vec).unwrap_or_default();
        let tail = |v: &Vec<P>| v.get(k..).map(<[P]>::to_vec).unwrap_or_default();
        let second = Stroke {
            a: node,
            b: s.b,
            centre: tail(&s.centre),
            left: tail(&s.left),
            right: tail(&s.right),
            column: s.column.get(k..).map(<[bool]>::to_vec).unwrap_or_default(),
            bend: bend.get(k..).map(<[bool]>::to_vec).unwrap_or_default(),
        };
        let first = Stroke {
            a: s.a,
            b: node,
            centre: head(&s.centre),
            left: head(&s.left),
            right: head(&s.right),
            column: s.column.get(..=k).map(<[bool]>::to_vec).unwrap_or_default(),
            bend: bend.get(..=k).map(<[bool]>::to_vec).unwrap_or_default(),
        };
        let q = s.centre.get(k).copied()?;
        let style = self.a.style.get(i).copied().flatten();
        self.a.continues.resize(self.a.strokes.len(), false);
        self.a.style.resize(self.a.strokes.len(), None);
        self.a.nodes.push(q);
        self.seen.push(false);
        if let Some(s) = self.a.strokes.get_mut(i) {
            *s = first;
        }
        self.a.strokes.push(second);
        // The column goes on through the cut: counted once.
        self.a.continues.push(true);
        self.a.style.push(style);
        self.sewn.push(false);
        self.way.push(false);
        Some(node)
    }

    /// Mark the shortest way along the centre lines of strokes not yet sewn from node `from` to
    /// node `to` (Dijkstra, bounded by the strokes and nodes): false when there is none.
    fn plan_way(&mut self, from: usize, to: usize) -> bool {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let n = self.a.nodes.len();
        let mut adj: Vec<Vec<(usize, usize, f64)>> = vec![vec![]; n];
        for (i, s) in self.a.strokes.iter().enumerate() {
            if !self.unsewn(i) || s.a == s.b {
                continue;
            }
            let len: f64 = s.centre.windows(2).map(|w| if let [p, q] = w { dist(*p, *q) } else { 0.0 }).sum();
            if !len.is_finite() {
                continue;
            }
            if let Some(v) = adj.get_mut(s.a) {
                v.push((s.b, i, len));
            }
            if let Some(v) = adj.get_mut(s.b) {
                v.push((s.a, i, len));
            }
        }
        let mut best = vec![f64::INFINITY; n];
        let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
        let mut heap = BinaryHeap::new();
        if let Some(b) = best.get_mut(from) {
            *b = 0.0;
        }
        // Lengths are finite and not negative: their bits sort as they do.
        heap.push(Reverse((0f64.to_bits(), from)));
        while let Some(Reverse((bits, u))) = heap.pop() {
            let d = f64::from_bits(bits);
            if d > best.get(u).copied().unwrap_or(f64::INFINITY) || u == to {
                continue;
            }
            for &(v, i, len) in adj.get(u).map(Vec::as_slice).unwrap_or(&[]) {
                let nd = d + len;
                if nd < best.get(v).copied().unwrap_or(f64::INFINITY) {
                    if let Some(b) = best.get_mut(v) {
                        *b = nd;
                    }
                    if let Some(p) = prev.get_mut(v) {
                        *p = Some((i, u));
                    }
                    heap.push(Reverse((nd.to_bits(), v)));
                }
            }
        }
        if !best.get(to).is_some_and(|d| d.is_finite()) {
            return false;
        }
        let mut at = to;
        for _ in 0..n {
            if at == from {
                return true;
            }
            let Some(Some((i, u))) = prev.get(at).copied() else { return false };
            if let Some(w) = self.way.get_mut(i) {
                *w = true;
            }
            at = u;
        }
        at == from
    }

    /// Running stitch along stroke `i`'s centre, forwards or back, at `length` mm.
    fn travel(&mut self, i: usize, forward: bool, length: f64) -> Result<(), String> {
        let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
        let (mut c, mut bend) = (s.centre.clone(), s.bend.clone());
        if !forward {
            c.reverse();
            bend.reverse();
        }
        // A running-stitch letter's travel follows its curves, as its runs do.
        let pts = if self.a.mode == LetterMode::Run { curve_run(&c, &bend, length) } else { run_stitches(&c, &bend, false, units(length)) };
        self.out.add(&pts)
    }

    /// Stroke `i` sewn as it shows: satin columns and runs, forwards or back.
    fn cover(&mut self, i: usize, forward: bool) -> Result<(), String> {
        let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
        let mut ps = pieces(s);
        if !forward {
            ps.reverse();
        }
        let own = self.a.style.get(i).copied().flatten();
        let rp = RailsParams {
            spacing: own.map_or(self.p.spacing, |o| o.spacing),
            max_stitch: self.p.max_stitch,
            compensation: own.map_or(self.p.compensation, |o| o.compensation),
        };
        for (col, from, to) in ps {
            let pts = if col {
                // A column split at a corner for routing is still one column.
                if !(from == 0 && self.a.continues.get(i).copied().unwrap_or(false)) {
                    self.columns += 1;
                }
                let (Some(l), Some(r)) = (s.left.get(from..=to), s.right.get(from..=to)) else { continue };
                let (mut l, mut r) = (l.to_vec(), r.to_vec());
                if !forward {
                    l.reverse();
                    r.reverse();
                }
                rails_satin(&l, &r, &rp, self.out.budget.saturating_sub(self.out.count))?
            } else {
                self.run_lines += 1;
                let Some(c) = s.centre.get(from..=to) else { continue };
                let (mut c, mut bend) = (c.to_vec(), s.bend.get(from..=to).map(<[bool]>::to_vec).unwrap_or_default());
                if !forward {
                    c.reverse();
                    bend.reverse();
                }
                let length = self.p.run_length.min(self.out.max);
                if self.a.mode == LetterMode::Run {
                    // Small letters: shorter stitches round curves, a bean run when asked.
                    let run = curve_run(&c, &bend, length);
                    if self.p.bean { bean(&run) } else { run }
                } else {
                    run_stitches(&c, &bend, false, units(length))
                }
            };
            self.out.add(&pts)?;
        }
        Ok(())
    }

    fn has_column(&self, i: usize) -> bool {
        self.a.strokes.get(i).is_some_and(|s| pieces(s).iter().any(|p| p.0))
    }

    /// The underlay stroke `i` takes (resolved by its columns' median width): none for running
    /// stitch and for strokes with no column.
    fn underlay(&self, i: usize) -> Underlay {
        if self.a.mode != LetterMode::Satin || !self.has_column(i) {
            return Underlay::None;
        }
        let kind = self.a.style.get(i).copied().flatten().map_or(self.p.underlay, |o| o.underlay);
        let Some(s) = self.a.strokes.get(i) else { return Underlay::None };
        let mut w: Vec<f64> = s.left.iter().zip(&s.right).zip(&s.column).filter(|(_, c)| **c).map(|((a, b), _)| dist(*a, *b)).collect();
        w.sort_by(f64::total_cmp);
        underlay_for(kind, w.get(w.len() / 2).copied().unwrap_or(0.0))
    }

    /// Underlay pass `pass` along stroke `i`'s columns, forwards or back; a centre pass runs
    /// along the whole centre line, as travel does.
    fn pass(&mut self, i: usize, pass: Pass, forward: bool, walk: f64) -> Result<(), String> {
        if pass == Pass::Centre {
            return self.travel(i, forward, walk);
        }
        let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
        let mut ps: Vec<(usize, usize)> = pieces(s).into_iter().filter(|p| p.0).map(|p| (p.1, p.2)).collect();
        if !forward {
            ps.reverse();
        }
        for (from, to) in ps {
            let (Some(l), Some(r)) = (s.left.get(from..=to), s.right.get(from..=to)) else { continue };
            let (mut l, mut r) = (l.to_vec(), r.to_vec());
            if !forward {
                l.reverse();
                r.reverse();
            }
            let pts = pass_stitches(&l, &r, pass, units(walk), units(self.out.max));
            self.out.add(&pts)?;
        }
        Ok(())
    }

    /// Sew the strokes reached from node `u`: those off the way out and back, then on along the
    /// way, if any; `back`: end at `u`. Depth is bounded by the number of strokes ([`MAX_EDGES`]
    /// and the dots of one letter).
    fn visit(&mut self, u: usize, back: bool) -> Result<(), String> {
        loop {
            let open = self.open_at(u);
            let far = |i: usize| {
                let s = self.a.strokes.get(i);
                let v = s.map_or(u, |s| if s.a == u { s.b } else { s.a });
                self.a.nodes.get(v).map_or(0.0, |q| dist(*q, self.target))
            };
            // Strokes that lead away from the next letter first; the way on last.
            let (i, last) = match open.iter().max_by(|x, y| far(**x).total_cmp(&far(**y))) {
                // Free to end anywhere (the last letter), its last stroke is sewn only on.
                Some(&i) => (i, self.free && !back && open.len() == 1),
                None => match (back, self.way_at(u)) {
                    (false, Some(i)) => (i, true),
                    _ => return Ok(()),
                },
            };
            if let Some(f) = self.sewn.get_mut(i) {
                *f = true;
            }
            let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
            let (forward, v) = if s.a == u { (true, s.b) } else { (false, s.a) };
            let under = self.underlay(i);
            // Held under the longest stitch, which rounding to plan units would otherwise pass.
            let walk = (if self.a.mode == LetterMode::Run { self.p.run_length } else { UNDERLAY_LENGTH_MM }).min(self.out.max);
            // A ring comes back where it started: sewn once round, as a last stroke is.
            if last || v == u {
                // The underlay's passes out and back, then the satin out.
                for (k, pass) in passes(under).iter().enumerate() {
                    self.pass(i, *pass, forward == (k % 2 == 0), walk)?;
                }
                self.cover(i, forward)?;
                if !last {
                    continue;
                }
                if let Some(f) = self.seen.get_mut(v) {
                    *f = true;
                }
                return self.visit(v, false);
            }
            self.travel(i, forward, walk)?;
            if !self.seen.get(v).copied().unwrap_or(true) {
                if let Some(f) = self.seen.get_mut(v) {
                    *f = true;
                }
                self.visit(v, true)?;
            }
            // The travel out was a centre run; an edge walk or a zigzag is sewn back and out
            // from here, then the satin back.
            if under != Underlay::Center {
                for (k, pass) in passes(under).iter().enumerate() {
                    self.pass(i, *pass, forward != (k % 2 == 0), walk)?;
                }
            }
            self.cover(i, !forward)?;
        }
    }
}

/// How much of a walk from `a` to `b` (mm) shows on the fabric: its length outside the ink of
/// the letter before (`before`) and of this one (`rings`), where satin covers it.
fn shown(a: P, b: P, before: &[Vec<P>], rings: &[Vec<P>]) -> f64 {
    let d = dist(a, b);
    if !d.is_finite() {
        return f64::INFINITY;
    }
    // Finite and bounded: at most HIDDEN_SAMPLES.
    let n = ((d / HIDDEN_STEP_MM).ceil().max(1.0) as usize).min(HIDDEN_SAMPLES);
    let out = (0..n)
        .filter(|i| {
            let t = (*i as f64 + 0.5) / n as f64;
            let q = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            !inside_rings(before, q) && !inside_rings(rings, q)
        })
        .count();
    d * out as f64 / n as f64
}

/// The most parts [`entry_part`] weighs.
const ENTRY_PARTS: usize = 8;

/// Of the [`ENTRY_PARTS`] parts still to sew nearest `m` (where the letter `before` ended), the one
/// a walk from `m` to its nearest point shows least of outside both letters' ink (`rings`); the
/// nearer when equal. One of its strokes.
fn entry_part(r: &Router<'_>, parts: &[Option<usize>], m: P, before: &[Vec<P>], rings: &[Vec<P>]) -> Option<usize> {
    // Each part's point nearest `m`: (distance, stroke, point), by the node naming the part.
    let mut near: Vec<Option<(f64, usize, P)>> = vec![None; r.a.nodes.len()];
    for (i, (s, pi)) in r.a.strokes.iter().zip(parts).enumerate() {
        let Some(slot) = pi.and_then(|pi| near.get_mut(pi)) else { continue };
        for q in &s.centre {
            let d = dist(*q, m);
            if d.is_finite() && slot.is_none_or(|b| d < b.0) {
                *slot = Some((d, i, *q));
            }
        }
    }
    let mut near: Vec<(f64, usize, P)> = near.into_iter().flatten().collect();
    near.sort_by(|x, y| x.0.total_cmp(&y.0));
    near.truncate(ENTRY_PARTS);
    near.into_iter().map(|(d, i, q)| (shown(m, q, before, rings), d, i)).min_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1))).map(|b| b.2)
}

/// The middle of the box of the part still to sew nearest part `part` (by the middles of their
/// boxes), if there is another.
fn next_part(r: &Router<'_>, parts: &[Option<usize>], part: usize) -> Option<P> {
    // Parts are named by a node: one box per node at most.
    let mut boxes: Vec<Option<(f64, f64, f64, f64)>> = vec![None; r.a.nodes.len()];
    for (s, pi) in r.a.strokes.iter().zip(parts) {
        let Some(b) = pi.and_then(|pi| boxes.get_mut(pi)) else { continue };
        for q in s.centre.iter().filter(|q| finite(**q)) {
            *b = Some(b.map_or((q.0, q.1, q.0, q.1), |b| (b.0.min(q.0), b.1.min(q.1), b.2.max(q.0), b.3.max(q.1))));
        }
    }
    let mid = |b: (f64, f64, f64, f64)| ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0);
    let own = mid((*boxes.get(part)?)?);
    boxes.iter().enumerate().filter_map(|(i, b)| b.filter(|_| i != part).map(mid)).min_by(|x, y| dist(*x, own).total_cmp(&dist(*y, own)))
}

/// Where the letter after one being sewn is.
struct Ahead<'a> {
    /// A point of it (its left middle, or bottom left when joined along the baseline); the
    /// letter's own right when it is the last.
    target: P,
    /// Its centre points, sampled ([`centre_sample`]): the letter is left at its point nearest
    /// them. Empty: nearest `target`.
    points: &'a [P],
    /// There is none: the letter may end anywhere.
    last: bool,
}

/// Sew one letter starting near `from` (where the letter `before` ended) and ending towards the
/// next letter (`ahead`): its fills first, then its strokes.
fn sew_letter(
    l: &Prepared,
    before: &[Vec<P>],
    p: &LetteringParams,
    from: Option<Pt>,
    ahead: &Ahead<'_>,
    budget: usize,
    connect: bool,
) -> Result<SewnLetter, String> {
    let target = ahead.target;
    let a = &l.a;
    let mut sewn = SewnLetter {
        rgb: l.rgb,
        word: l.word,
        mode: a.mode,
        runs: vec![],
        columns: 0,
        run_lines: 0,
        fills: a.fills.len(),
        joined: false,
        lead_mm: 0.0,
    };
    // The stitch length of a walk to a part or from the last letter: running stitch at its own
    // length, else the underlay's, held under the longest stitch.
    let walk = (if a.mode == LetterMode::Run { p.run_length } else { UNDERLAY_LENGTH_MM }).min((p.max_stitch - 0.15).max(0.3));
    let mut r = Router {
        a: a.clone(),
        p,
        sewn: vec![false; a.strokes.len()],
        seen: vec![false; a.nodes.len()],
        way: vec![false; a.strokes.len()],
        free: false,
        target,
        out: Out { runs: vec![], budget, max: (p.max_stitch - 0.15).max(0.3), count: 0 },
        columns: 0,
        run_lines: 0,
    };
    // A connector from the letter before is walked when short, hidden in this letter's ink, or
    // when letters are never trimmed; else it is a trim and a jump.
    let walks = |m: P, q: P| p.trims == LetterTrims::None || shown(m, q, before, &l.rings) <= MAX_CONNECTOR_MM;
    let fp = FillParams { max_stitch: p.max_stitch, ..p.fill };
    for rings in &a.fills {
        for mut run in tatami(rings, FillRule::NonZero, &fp, budget.saturating_sub(r.out.count))? {
            if let (true, true, Some(q), Some(&b)) = (r.out.runs.is_empty(), connect, from, run.first())
                && walks(q.to_mm(), b.to_mm())
            {
                // Walked to from the last letter: its stitches lead the first run.
                sewn.lead_mm = shown(q.to_mm(), b.to_mm(), before, &l.rings);
                let mut lead = line_run(q.to_mm(), b.to_mm(), walk);
                lead.append(&mut run);
                run = lead;
                sewn.joined = true;
            }
            r.out.count = r.out.count.saturating_add(run.len());
            if r.out.count > budget {
                return Err(TOO_MANY_STEPS.into());
            }
            r.out.runs.push(run);
        }
    }
    let mut at = r.out.pos().map(|q| q.to_mm()).or(from.map(|q| q.to_mm()));
    // Joined along the baseline, the letter's first part is the one at its bottom left.
    let bottom_left = bounds(&l.rings).map(|b| (b.0, b.3)).filter(|_| p.join == LetterJoin::Bottom);
    // Each round sews a part, and cutting strokes makes no new part: bounded by the strokes.
    for _ in 0..=a.strokes.len() {
        let parts = r.parts();
        // The next part is the one nearest the needle (closest join); the letter's first, with no
        // needle, its leftmost, or the one at its bottom left when joined along the baseline.
        let towards = if r.out.runs.is_empty() { bottom_left.or(at) } else { at };
        let Some((mut si, _)) = r.nearest(&parts, None, |q| towards.map_or(q.0, |m| dist(q, m))) else { break };
        // Walked to from the letter before (closest join): of the parts about as near, the one the
        // walk shows least of (an i's stem, not its dot beside it).
        if let (true, true, LetterJoin::Closest, Some(m)) = (r.out.runs.is_empty(), connect, p.join, at)
            && let Some(i) = entry_part(&r, &parts, m, before, &l.rings)
        {
            si = i;
        }
        let Some(part) = parts.get(si).copied().flatten() else { break };
        // It is left at its point nearest the next part still to sew, else nearest the next
        // letter; travelling to there under its stitching rather than across the gap. The last
        // part of the last letter ends where its strokes do.
        let aim = next_part(&r, &parts, part);
        r.free = ahead.last && aim.is_none();
        let exit_at = match aim {
            _ if r.free => None,
            Some(aim) => r.nearest(&parts, Some(part), |q| dist(q, aim)),
            // The last part: at its point nearest the next letter's strokes.
            None if !ahead.points.is_empty() => {
                r.nearest(&parts, Some(part), |q| ahead.points.iter().map(|n| dist(q, *n)).fold(f64::INFINITY, f64::min))
            }
            None => r.nearest(&parts, Some(part), |q| dist(q, target)),
        };
        let exit = match exit_at {
            Some((xi, xk)) => Some(r.cut(xi, xk, true).ok_or("a stroke went missing")?),
            None => None,
        };
        let parts = r.parts();
        let part =
            r.a.strokes.iter().position(|s| exit.is_some_and(|x| s.a == x || s.b == x)).or(Some(si)).and_then(|i| parts.get(i).copied().flatten());
        // It is entered at its point nearest the needle (with none, its leftmost).
        let entry = part.and_then(|part| r.nearest(&parts, Some(part), |q| towards.map_or(q.0, |m| dist(q, m))));
        let here = entry.and_then(|(i, k)| r.a.strokes.get(i).and_then(|s| s.centre.get(k)).copied());
        let walked = match (at, here) {
            // From the last letter, when the letters are joined: walk there, stitching.
            (Some(m), Some(q)) if r.out.runs.is_empty() && connect && walks(m, q) => {
                r.out.runs.push(vec![]);
                r.out.add(&line_run(m, q, walk))?;
                sewn.joined = true;
                sewn.lead_mm = shown(m, q, before, &l.rings);
                true
            }
            // Another part of this letter within a stitch, or reached inside the letter's ink
            // (under the satin sewn after it): walk there, stitching.
            (Some(m), Some(q)) if !r.out.runs.is_empty() && (dist(m, q) <= NO_TRIM_GAP_MM || hidden(m, q, &l.rings)) => {
                r.out.add(&line_run(m, q, walk))?;
                true
            }
            // Else a trim and a jump reach it (the first run: unless the letters are joined),
            // straight to where it is left.
            _ => {
                r.out.runs.push(vec![]);
                false
            }
        };
        // Walked to: on from the entry along the strokes to the exit, the rest of the part sewn
        // out and back from the nodes on the way.
        // Ending anywhere, it starts at the entry (jumped to or not).
        let start = match (exit, entry) {
            (Some(exit), Some((i, k))) if walked => r.cut(i, k, false).unwrap_or(exit),
            (Some(exit), _) => exit,
            (None, Some((i, k))) => r.cut(i, k, true).ok_or("a stroke went missing")?,
            (None, None) => break,
        };
        if let Some(exit) = exit
            && start != exit
            && !r.plan_way(start, exit)
        {
            r.way.iter_mut().for_each(|w| *w = false);
        }
        if let Some(f) = r.seen.get_mut(start) {
            *f = true;
        }
        let before_count = r.sewn.iter().filter(|s| **s).count();
        r.visit(start, false)?;
        r.way.iter_mut().for_each(|w| *w = false);
        at = r.out.pos().map(|q| q.to_mm());
        if r.sewn.iter().filter(|s| **s).count() == before_count {
            break;
        }
    }
    sewn.runs = r.out.runs.into_iter().filter(|run| run.len() > 1).collect();
    sewn.columns = r.columns;
    sewn.run_lines = r.run_lines;
    Ok(sewn)
}

/// A running-stitch letter's centre line smoothed further: its skeleton is only a few pixels
/// wide and steps show at a short stitch. Its ends and corners stay where they are.
fn smooth_run(s: &mut Stroke) {
    let n = s.centre.len();
    let reach = (RUN_CORNER_REACH_MM / columns::STEP_MM).ceil() as usize;
    for i in 0..n {
        if !s.bend.get(i).copied().unwrap_or(false) {
            continue;
        }
        let (Some(a), Some(b), Some(c)) = (s.centre.get(i.saturating_sub(reach)), s.centre.get(i), s.centre.get((i + reach).min(n - 1))) else {
            continue;
        };
        let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
        let (lu, lv) = (u.0.hypot(u.1), v.0.hypot(v.1));
        let turn = if lu > 1e-9 && lv > 1e-9 { ((u.0 * v.0 + u.1 * v.1) / (lu * lv)).clamp(-1.0, 1.0).acos().to_degrees() } else { 0.0 };
        if turn < RUN_CORNER_TURN_DEG
            && let Some(f) = s.bend.get_mut(i)
        {
            *f = false;
        }
    }
    for _ in 0..RUN_SMOOTHING {
        let prev = s.centre.clone();
        for i in 1..prev.len().saturating_sub(1) {
            if s.bend.get(i).copied().unwrap_or(false) {
                continue;
            }
            if let (Some(a), Some(b), Some(c), Some(o)) = (prev.get(i - 1), prev.get(i), prev.get(i + 1), s.centre.get_mut(i)) {
                *o = ((a.0 + 2.0 * b.0 + c.0) / 4.0, (a.1 + 2.0 * b.1 + c.1) / 4.0);
            }
        }
    }
}

/// Running stitch along `line` (mm) at most `length` mm a stitch, shorter where it bends so no
/// stitch strays more than [`RUN_CHORD_MM`] from it (down to [`RUN_MIN_MM`]); corners (`bend`)
/// and both ends are needle points.
fn curve_run(line: &[P], bend: &[bool], length: f64) -> Vec<Pt> {
    let mut out: Vec<Pt> = vec![];
    let mut push = |q: P| {
        if let Some(p) = Pt::from_mm(q.0, q.1)
            && out.last() != Some(&p)
        {
            out.push(p);
        }
    };
    let Some(&first) = line.first() else { return out };
    push(first);
    let (mut i, n) = (0usize, line.len());
    while i + 1 < n {
        let Some(&a) = line.get(i) else { break };
        // The furthest point within the length whose chord keeps to the line, stopping at a corner.
        let mut j = i + 1;
        while j + 1 < n && !bend.get(j).copied().unwrap_or(false) {
            let Some(&b) = line.get(j + 1) else { break };
            let d = dist(a, b);
            if d > length {
                break;
            }
            let strays = line.get(i + 1..=j).is_some_and(|mid| mid.iter().any(|m| crate::fill::seg_dist(*m, a, b) > RUN_CHORD_MM));
            if strays && dist(a, line.get(j).copied().unwrap_or(a)) >= RUN_MIN_MM {
                break;
            }
            j += 1;
        }
        let Some(&b) = line.get(j) else { break };
        // A straight stretch longer than the length is cut evenly.
        let d = dist(a, b);
        let k = (d / length).ceil().clamp(1.0, 1000.0) as usize;
        for m in 1..=k {
            let t = m as f64 / k as f64;
            push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        i = j;
    }
    out
}

/// `pts` as a bean run: each stitch sewn forward, back and forward again.
fn bean(pts: &[Pt]) -> Vec<Pt> {
    let mut out = Vec::with_capacity(pts.len() * 3);
    for w in pts.windows(2) {
        if let [a, b] = w {
            if out.is_empty() {
                out.push(*a);
            }
            out.extend([*b, *a, *b]);
        }
    }
    if out.is_empty() {
        out.extend(pts.first());
    }
    out
}

/// Running stitch along the straight line from `a` to `b` (mm), both ends included, at `step` mm.
fn line_run(a: P, b: P, step: f64) -> Vec<Pt> {
    run_stitches(&[a, b], &[], false, units(step))
}

/// Whether `q` lies inside `rings` (non-zero winding, as glyph outlines are filled) or on an edge.
fn inside_rings(rings: &[Vec<P>], q: P) -> bool {
    let mut w = 0i64;
    for ring in rings {
        for (i, &a) in ring.iter().enumerate() {
            let b = ring.get(i + 1).or(ring.first()).copied().unwrap_or(a);
            if !(finite(a) && finite(b)) {
                continue;
            }
            if (a.1 <= q.1) != (b.1 <= q.1) {
                let t = (q.1 - a.1) / (b.1 - a.1);
                let u = a.0 + t * (b.0 - a.0);
                if u > q.0 {
                    w += if b.1 > a.1 { 1 } else { -1 };
                }
            }
        }
    }
    w != 0
}

/// Whether a straight walk from `a` to `b` (mm) stays inside the ink of `rings`, so the satin
/// sewn over it later hides it: every [`HIDDEN_STEP_MM`] along it is inside.
pub(crate) fn hidden(a: P, b: P, rings: &[Vec<P>]) -> bool {
    let d = dist(a, b);
    if !d.is_finite() {
        return false;
    }
    // Finite and bounded: at most HIDDEN_SAMPLES.
    let n = ((d / HIDDEN_STEP_MM).ceil().max(1.0) as usize).min(HIDDEN_SAMPLES);
    (0..=n).all(|i| {
        let t = i as f64 / n as f64;
        inside_rings(rings, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t))
    })
}

/// The box of a letter's outline: (min x, min y, max x, max y).
fn bounds(rings: &[Vec<P>]) -> Option<(f64, f64, f64, f64)> {
    let mut it = rings.iter().flatten().filter(|q| finite(**q));
    let first = it.next()?;
    Some(it.fold((first.0, first.1, first.0, first.1), |b, q| (b.0.min(q.0), b.1.min(q.1), b.2.max(q.0), b.3.max(q.1))))
}

/// Sew `letters` in order, all in `mode` (satin or run, from [`mode_for`]; a letter too wide
/// for satin is filled), each started at its point nearest where the last one ended. At most
/// [`MAX_LETTERS`] letters and `budget` needle points.
pub fn sew_letters(letters: &[Letter], mode: LetterMode, p: &LetteringParams, budget: usize) -> Result<Vec<SewnLetter>, String> {
    if letters.len() > MAX_LETTERS {
        return Err(format!("lettering sews at most {MAX_LETTERS} letters"));
    }
    let prepared = letters
        .iter()
        .map(|l| {
            let mut a = analyse(&l.rings, mode, p)?;
            split_corners(&mut a);
            Ok(Prepared { rings: l.rings.clone(), rgb: l.rgb, word: l.word, a })
        })
        .collect::<Result<Vec<_>, String>>()?;
    sew_prepared(&prepared, p, budget)
}

/// Sew converted letters (`text.toStitches`) as lettering again: each letter's pieces gathered
/// back into strokes (piece ends that meet are nodes), its fills first, joined and trimmed as
/// [`sew_letters`] joins and trims. At most [`MAX_LETTERS`] letters and `budget` needle points.
pub fn sew_pieces(letters: &[PieceLetter], p: &LetteringParams, budget: usize) -> Result<Vec<SewnLetter>, String> {
    if letters.len() > MAX_LETTERS {
        return Err(format!("lettering sews at most {MAX_LETTERS} letters"));
    }
    let prepared = letters.iter().map(from_pieces).collect::<Result<Vec<_>, String>>()?;
    sew_prepared(&prepared, p, budget)
}

/// The most centre points of a letter that choosing where the letter before it ends looks at.
const AHEAD_POINTS: usize = 64;

/// Up to [`AHEAD_POINTS`] centre points of `a`'s strokes, evenly picked.
fn centre_sample(a: &Analysis) -> Vec<P> {
    let n: usize = a.strokes.iter().map(|s| s.centre.len()).sum();
    let step = n.div_ceil(AHEAD_POINTS).max(1);
    a.strokes.iter().flat_map(|s| s.centre.iter().copied()).step_by(step).filter(|q| finite(*q)).collect()
}

/// Sew `letters` in order, each started at its point nearest where the last one ended.
fn sew_prepared(letters: &[Prepared], p: &LetteringParams, budget: usize) -> Result<Vec<SewnLetter>, String> {
    let budget = budget.min(MAX_STEPS);
    let mut out: Vec<SewnLetter> = Vec::with_capacity(letters.len());
    let mut pos: Option<Pt> = None;
    let mut word: Option<usize> = None;
    let mut used = 0usize;
    for (i, l) in letters.iter().enumerate() {
        // Towards the left middle of the next letter, or this one's right; joined along the
        // baseline, towards their bottom (page y grows down).
        let height = |b: (f64, f64, f64, f64)| if p.join == LetterJoin::Bottom { b.3 } else { (b.1 + b.3) / 2.0 };
        let target = match letters.get(i + 1).and_then(|n| bounds(&n.rings)) {
            Some(b) => (b.0, height(b)),
            None => bounds(&l.rings).map_or((0.0, 0.0), |b| (b.2, height(b))),
        };
        // Joined to the letter before it by a walk of stitches, never by a bare jump, which a
        // machine sews as a loose thread across the fabric: within a word by default.
        let connect = pos.is_some()
            && match p.trims {
                LetterTrims::None => true,
                LetterTrims::Word => word == Some(l.word),
                LetterTrims::Letter => false,
            };
        let before = i.checked_sub(1).and_then(|j| letters.get(j)).map_or(&[][..], |b| &b.rings[..]);
        // Joined where closest, the letter is left at its point nearest the next one's strokes.
        let points = match letters.get(i + 1) {
            Some(n) if p.join == LetterJoin::Closest => centre_sample(&n.a),
            _ => vec![],
        };
        let ahead = Ahead { target, points: &points, last: i + 1 == letters.len() };
        let s = sew_letter(l, before, p, pos, &ahead, budget.saturating_sub(used), connect)?;
        word = Some(l.word);
        used = used.saturating_add(s.runs.iter().map(Vec::len).sum());
        if used > budget {
            return Err(TOO_MANY_STEPS.into());
        }
        if let Some(q) = s.runs.last().and_then(|r| r.last()) {
            pos = Some(*q);
        }
        out.push(s);
    }
    Ok(out)
}
