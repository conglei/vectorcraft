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
//! later; letters are joined where the needle is nearest, and trimmed per word by default.

#[cfg(test)]
mod tests;

use vectorcraft_doc::LetterTrims;
use vectorcraft_geom::FillRule;

use crate::autodigitize::{Split, split_wide};
use crate::columns::{self, Stroke, dist, extend_nodes, finite, pieces};
use crate::fill::{FillParams, tatami};
use crate::run::run_stitches;
use crate::satin::{RailsParams, rails_satin};
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
    /// A centre run under satin columns.
    pub underlay: bool,
    pub trims: LetterTrims,
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
            underlay: true,
            trims: LetterTrims::Word,
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
struct Analysis {
    mode: LetterMode,
    nodes: Vec<P>,
    strokes: Vec<Stroke>,
    /// Each stroke's own satin settings (a converted letter's column), else the lettering's.
    style: Vec<Option<PieceStyle>>,
    /// Outlines (rings, nonzero rule) sewn as tatami before the strokes.
    fills: Vec<Vec<Vec<P>>>,
}

/// How one column of a converted letter sews, from its own stitch properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceStyle {
    /// Satin spacing, peak to peak, mm.
    pub spacing: f64,
    /// Pull compensation each side, mm.
    pub compensation: f64,
    /// A centre run under it.
    pub underlay: bool,
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
        Analysis { mode, nodes: vec![], strokes: vec![], style: vec![], fills }
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
                    Analysis { mode, nodes: extend_nodes(nodes, &stretches), strokes: stretches, style, fills: vec![fill] }
                }
                Split::Narrow | Split::Whole => empty(LetterMode::Fill),
            });
        }
    }
    let style = vec![None; strokes.len()];
    Ok(Analysis { mode, nodes: extend_nodes(nodes, &strokes), strokes, style, fills: vec![] })
}

/// Piece ends of a converted letter are one node when closer than this many half widths of the
/// column ending there (a column's end is the middle of its last rung, which lies off the centre
/// line where a rail reaches into another stroke).
const SAME_NODE: f64 = 0.8;
/// ... and always when closer than this, mm.
const SAME_POINT_MM: f64 = 0.05;

/// Whether each point of `pts` is a corner: the line turns there by more than 30 degrees.
fn bends(pts: &[P]) -> Vec<bool> {
    (0..pts.len())
        .map(|i| {
            let (Some(a), Some(b), Some(c)) = (i.checked_sub(1).and_then(|j| pts.get(j)), pts.get(i), pts.get(i + 1)) else { return false };
            let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
            let (lu, lv) = (u.0.hypot(u.1), v.0.hypot(v.1));
            lu > 1e-9 && lv > 1e-9 && (u.0 * v.0 + u.1 * v.1) / (lu * lv) < 0.866
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
    let a = Analysis { mode, nodes: extend_nodes(nodes, &strokes), strokes, style, fills: l.fills.clone() };
    Ok(Prepared { rings, rgb: l.rgb, word: l.word, a })
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
    a: &'a Analysis,
    p: &'a LetteringParams,
    sewn: Vec<bool>,
    seen: Vec<bool>,
    target: P,
    out: Out,
    columns: usize,
    run_lines: usize,
}

impl Router<'_> {
    /// The strokes at node `u` not yet sewn.
    fn open_at(&self, u: usize) -> Vec<usize> {
        self.a
            .strokes
            .iter()
            .enumerate()
            .filter(|(i, s)| (s.a == u || s.b == u) && !self.sewn.get(*i).copied().unwrap_or(true))
            .map(|(i, _)| i)
            .collect()
    }

    /// Running stitch along stroke `i`'s centre, forwards or back, at `length` mm.
    fn travel(&mut self, i: usize, forward: bool, length: f64) -> Result<(), String> {
        let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
        let (mut c, mut bend) = (s.centre.clone(), s.bend.clone());
        if !forward {
            c.reverse();
            bend.reverse();
        }
        let pts = run_stitches(&c, &bend, false, units(length));
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
                self.columns += 1;
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
                run_stitches(&c, &bend, false, units(self.p.run_length.min(self.out.max)))
            };
            self.out.add(&pts)?;
        }
        Ok(())
    }

    fn has_column(&self, i: usize) -> bool {
        self.a.strokes.get(i).is_some_and(|s| pieces(s).iter().any(|p| p.0))
    }

    /// Sew the strokes reached from node `u`; `back`: end at `u`. Depth is bounded by the
    /// number of strokes ([`MAX_EDGES`] and the dots of one letter).
    fn visit(&mut self, u: usize, back: bool) -> Result<(), String> {
        loop {
            let open = self.open_at(u);
            if open.is_empty() {
                return Ok(());
            }
            let far = |i: usize| {
                let s = self.a.strokes.get(i);
                let v = s.map_or(u, |s| if s.a == u { s.b } else { s.a });
                self.a.nodes.get(v).map_or(0.0, |q| dist(*q, self.target))
            };
            // Strokes that lead away from the next letter first; the one towards it last.
            let Some(&i) = open.iter().max_by(|x, y| far(**x).total_cmp(&far(**y))) else { return Ok(()) };
            let last = open.len() == 1 && !back;
            if let Some(f) = self.sewn.get_mut(i) {
                *f = true;
            }
            let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
            let (forward, v) = if s.a == u { (true, s.b) } else { (false, s.a) };
            let underlay = self.a.style.get(i).copied().flatten().map_or(self.p.underlay, |o| o.underlay);
            let under = underlay && self.a.mode == LetterMode::Satin && self.has_column(i);
            // Held under the longest stitch, which rounding to plan units would otherwise pass.
            let walk = (if self.a.mode == LetterMode::Run { self.p.run_length } else { UNDERLAY_LENGTH_MM }).min(self.out.max);
            if last {
                if under {
                    self.travel(i, forward, walk)?;
                    self.travel(i, !forward, walk)?;
                }
                self.cover(i, forward)?;
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
            self.cover(i, !forward)?;
        }
    }
}

/// Sew one letter starting near `from` and ending towards `target`: its fills first, then its
/// strokes.
fn sew_letter(l: &Prepared, p: &LetteringParams, from: Option<Pt>, target: P, budget: usize, connect: bool) -> Result<SewnLetter, String> {
    let a = &l.a;
    let mut sewn = SewnLetter { rgb: l.rgb, word: l.word, mode: a.mode, runs: vec![], columns: 0, run_lines: 0, fills: a.fills.len(), joined: false };
    // The stitch length of a walk to a part or from the last letter: running stitch at its own
    // length, else the underlay's, held under the longest stitch.
    let walk = (if a.mode == LetterMode::Run { p.run_length } else { UNDERLAY_LENGTH_MM }).min((p.max_stitch - 0.15).max(0.3));
    let mut r = Router {
        a,
        p,
        sewn: vec![false; a.strokes.len()],
        seen: vec![false; a.nodes.len()],
        target,
        out: Out { runs: vec![], budget, max: (p.max_stitch - 0.15).max(0.3), count: 0 },
        columns: 0,
        run_lines: 0,
    };
    let fp = FillParams { max_stitch: p.max_stitch, ..p.fill };
    for rings in &a.fills {
        for mut run in tatami(rings, FillRule::NonZero, &fp, budget.saturating_sub(r.out.count))? {
            if let (true, true, Some(q), Some(&b)) = (r.out.runs.is_empty(), connect, from, run.first()) {
                // Walked to from the last letter: its stitches lead the first run.
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
    while r.sewn.iter().any(|s| !s) {
        // The next part starts at its node nearest the needle (closest join).
        let open: Vec<usize> = (0..a.nodes.len()).filter(|&n| !r.open_at(n).is_empty()).collect();
        let near = |n: &usize| a.nodes.get(*n).map_or(f64::INFINITY, |q| at.map_or(q.0, |m| dist(*q, m)));
        let Some(&start) = open.iter().min_by(|x, y| near(x).total_cmp(&near(y))) else { break };
        if let Some(f) = r.seen.get_mut(start) {
            *f = true;
        }
        let here = a.nodes.get(start).copied();
        match (at, here) {
            // From the last letter, when the letters are joined: walk there, stitching.
            (Some(m), Some(q)) if r.out.runs.is_empty() && connect => {
                r.out.runs.push(vec![]);
                r.out.add(&line_run(m, q, walk))?;
                sewn.joined = true;
            }
            // Another part of this letter within a stitch, or reached inside the letter's ink
            // (under the satin sewn after it): walk there, stitching.
            (Some(m), Some(q)) if !r.out.runs.is_empty() && (dist(m, q) <= NO_TRIM_GAP_MM || hidden(m, q, &l.rings)) => {
                r.out.add(&line_run(m, q, walk))?;
            }
            // Else a trim and a jump reach it (the first run: unless the letters are joined).
            _ => r.out.runs.push(vec![]),
        }
        r.visit(start, false)?;
        at = r.out.pos().map(|q| q.to_mm());
    }
    sewn.runs = r.out.runs.into_iter().filter(|run| run.len() > 1).collect();
    sewn.columns = r.columns;
    sewn.run_lines = r.run_lines;
    Ok(sewn)
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
        .map(|l| Ok(Prepared { rings: l.rings.clone(), rgb: l.rgb, word: l.word, a: analyse(&l.rings, mode, p)? }))
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

/// Sew `letters` in order, each started at its point nearest where the last one ended.
fn sew_prepared(letters: &[Prepared], p: &LetteringParams, budget: usize) -> Result<Vec<SewnLetter>, String> {
    let budget = budget.min(MAX_STEPS);
    let mut out: Vec<SewnLetter> = Vec::with_capacity(letters.len());
    let mut pos: Option<Pt> = None;
    let mut word: Option<usize> = None;
    let mut used = 0usize;
    for (i, l) in letters.iter().enumerate() {
        // Towards the left middle of the next letter, or this one's right.
        let target = match letters.get(i + 1).and_then(|n| bounds(&n.rings)) {
            Some(b) => (b.0, (b.1 + b.3) / 2.0),
            None => bounds(&l.rings).map_or((0.0, 0.0), |b| (b.2, (b.1 + b.3) / 2.0)),
        };
        // Joined to the letter before it by a walk of stitches, never by a bare jump, which a
        // machine sews as a loose thread across the fabric: within a word by default.
        let connect = pos.is_some()
            && match p.trims {
                LetterTrims::None => true,
                LetterTrims::Word => word == Some(l.word),
                LetterTrims::Letter => false,
            };
        let s = sew_letter(l, p, pos, target, budget.saturating_sub(used), connect)?;
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
