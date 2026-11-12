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

mod skeleton;
#[cfg(test)]
mod tests;

use vectorcraft_doc::LetterTrims;
use vectorcraft_geom::FillRule;

use crate::fill::{FillParams, tatami};
use crate::run::run_stitches;
use crate::satin::{RailsParams, rails_satin, resample};
use crate::{MAX_STEPS, Pt, TOO_MANY_STEPS, UNITS_PER_MM};
use skeleton::{Graph, Grid};

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
/// The most letters one text object sews.
pub const MAX_LETTERS: usize = 10_000;
/// Centre line sampling, mm.
const STEP_MM: f64 = 0.25;
/// The most strokes one glyph may have; more (a hostile or very ornate outline) is sewn as fill.
const MAX_EDGES: usize = 400;
/// Spurs shorter than this many junction half widths are thinning leftovers.
const SPUR: f64 = 1.2;
/// Rails are held to this many local half widths from the centre line.
const RAIL_HOLD: f64 = 1.6;
/// Free ends stop this far inside the outline, mm.
const END_INSET_MM: f64 = 0.1;

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
    /// The narrowest rung, mm.
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
    /// The outline, when the letter is filled.
    pub fill: Vec<Vec<P>>,
}

/// A sewn letter.
#[derive(Clone, Debug, PartialEq)]
pub struct SewnLetter {
    pub rgb: [u8; 3],
    pub word: usize,
    pub mode: LetterMode,
    /// Needle points; the first run follows the letter before it, the others (separate parts of
    /// the letter) are reached by a jump.
    pub runs: Vec<Vec<Pt>>,
    pub columns: usize,
    pub run_lines: usize,
}

fn dist(a: P, b: P) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn finite(p: P) -> bool {
    p.0.is_finite() && p.1.is_finite()
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

/// A stroke ready to sew: its centre line from node `a` to node `b`, its rails, and whether each
/// point is part of a satin column.
#[derive(Clone, Debug)]
struct Stroke {
    a: usize,
    b: usize,
    centre: Vec<P>,
    left: Vec<P>,
    right: Vec<P>,
    column: Vec<bool>,
}

/// Stretches of one kind: (column?, from, to) point indices. Every rung of a column piece is a
/// column point; run pieces reach to the end points of their neighbours, so the line stays
/// continuous.
fn pieces(s: &Stroke) -> Vec<(bool, usize, usize)> {
    let n = s.centre.len();
    let flag = |i: usize| s.column.get(i).copied().unwrap_or(false);
    let mut blocks: Vec<(bool, usize, usize)> = vec![];
    let mut start = 0usize;
    for i in 1..=n {
        if i == n || flag(i) != flag(start) {
            // A column too short to sew as satin (under three rungs) is sewn as a run.
            let col = flag(start) && i >= start + 3;
            match blocks.last_mut() {
                Some(b) if b.0 == col => b.2 = i - 1,
                _ => blocks.push((col, start, i - 1)),
            }
            start = i;
        }
    }
    let last = n.saturating_sub(1);
    blocks
        .into_iter()
        .map(|(col, a, b)| if col { (col, a, b) } else { (col, a.saturating_sub(1), (b + 1).min(last)) })
        .filter(|p| p.2 > p.1)
        .collect()
}

/// A letter analysed: its strokes and nodes, or its mode when it has no strokes to sew.
struct Analysis {
    mode: LetterMode,
    nodes: Vec<P>,
    strokes: Vec<Stroke>,
}

/// The nearest hit of the ray from `o` along `d` (a unit vector) with `segs`.
fn ray(segs: &[(P, P)], o: P, d: P) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &(a, b) in segs {
        let e = (b.0 - a.0, b.1 - a.1);
        let den = d.0 * e.1 - d.1 * e.0;
        if den.abs() < 1e-12 {
            continue;
        }
        let w = (a.0 - o.0, a.1 - o.1);
        let t = (w.0 * e.1 - w.1 * e.0) / den;
        let u = (w.0 * d.1 - w.1 * d.0) / den;
        if t > 1e-9 && (-1e-9..=1.0 + 1e-9).contains(&u) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best
}

/// `pts` smoothed (ends kept) and resampled every [`STEP_MM`].
fn smooth(pts: &[P]) -> Vec<P> {
    let mut v = pts.to_vec();
    for _ in 0..4 {
        let prev = v.clone();
        for i in 1..prev.len().saturating_sub(1) {
            if let (Some(a), Some(b), Some(c), Some(o)) = (prev.get(i - 1), prev.get(i), prev.get(i + 1), v.get_mut(i)) {
                *o = ((a.0 + 2.0 * b.0 + c.0) / 4.0, (a.1 + 2.0 * b.1 + c.1) / 4.0);
            }
        }
    }
    let len: f64 = v.windows(2).map(|w| if let [a, b] = w { dist(*a, *b) } else { 0.0 }).sum();
    // At most a few thousand: the grid is at most 800 pixels a side.
    let n = ((len / STEP_MM).ceil() as usize).clamp(1, 20_000) + 1;
    resample(&v, n)
}

/// The unit tangent at point `i` of `pts`.
fn tangent(pts: &[P], i: usize) -> P {
    let a = pts.get(i.saturating_sub(1)).or(pts.get(i));
    let b = pts.get(i + 1).or(pts.get(i));
    match (a, b) {
        (Some(a), Some(b)) => {
            let l = dist(*a, *b);
            if l > 1e-12 { ((b.0 - a.0) / l, (b.1 - a.1) / l) } else { (1.0, 0.0) }
        }
        _ => (1.0, 0.0),
    }
}

/// Extend `pts` past its last point along its tangent to just inside the outline, at most
/// `reach`.
fn extend_end(pts: &mut Vec<P>, segs: &[(P, P)], reach: f64) {
    let n = pts.len();
    let (Some(&end), Some(&back)) = (pts.last(), pts.get(n.saturating_sub(1 + (reach / STEP_MM).ceil().clamp(1.0, 8.0) as usize))) else { return };
    let l = dist(back, end);
    if l < 1e-9 {
        return;
    }
    let d = ((end.0 - back.0) / l, (end.1 - back.1) / l);
    let Some(t) = ray(segs, end, d) else { return };
    let go = (t - END_INSET_MM).min(reach);
    if go <= STEP_MM / 2.0 {
        return;
    }
    let k = (go / STEP_MM).ceil().clamp(1.0, 100.0) as usize;
    for i in 1..=k {
        let f = go * i as f64 / k as f64;
        pts.push((end.0 + d.0 * f, end.1 + d.1 * f));
    }
}

/// The analysis of a glyph outline in `mode` (satin or run). Hostile outlines are bounded: more
/// than [`skeleton::MAX_SEGMENTS`] segments is an error, more strokes than [`MAX_EDGES`] sews the
/// letter as fill.
fn analyse(rings: &[Vec<P>], mode: LetterMode, p: &LetteringParams) -> Result<Analysis, String> {
    if rings.iter().flatten().any(|q| !finite(*q)) {
        return Err("the letter's outline has points that aren't numbers".into());
    }
    let segs = skeleton::segments(rings);
    if segs.len() > skeleton::MAX_SEGMENTS {
        return Err("the letter's outline has too many points".into());
    }
    let empty = |mode| Analysis { mode, nodes: vec![], strokes: vec![] };
    if mode == LetterMode::Fill {
        return Ok(empty(LetterMode::Fill));
    }
    let Some(grid) = skeleton::rasterize(&segs) else { return Ok(empty(mode)) };
    let skel = skeleton::thin(&grid);
    let Some(mut gr) = skeleton::graph(&grid, &skel, MAX_EDGES) else { return Ok(empty(LetterMode::Fill)) };
    skeleton::prune(&mut gr, SPUR, grid.px);
    let strokes = strokes(&gr, &grid, &segs, mode, p);
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
            return Ok(empty(LetterMode::Fill));
        }
    }
    Ok(Analysis { mode, nodes: extend_nodes(nodes, &strokes), strokes })
}

/// The node positions, with room for the nodes [`strokes`] added for dots.
fn extend_nodes(mut nodes: Vec<P>, strokes: &[Stroke]) -> Vec<P> {
    for s in strokes {
        for (n, q) in [(s.a, s.centre.first()), (s.b, s.centre.last())] {
            while nodes.len() <= n {
                nodes.push(q.copied().unwrap_or((0.0, 0.0)));
            }
        }
    }
    nodes
}

/// The strokes of graph `gr`: centre lines smoothed, free ends extended to the outline, rails
/// cast to the outline; a dot (a node with no strokes) becomes a short stroke across it.
fn strokes(gr: &Graph, grid: &Grid, segs: &[(P, P)], mode: LetterMode, p: &LetteringParams) -> Vec<Stroke> {
    let mut out = vec![];
    let mut next_node = gr.nodes.len();
    let mut lines: Vec<(usize, usize, Vec<P>)> = vec![];
    for (_, e) in gr.live() {
        let mut c = smooth(&e.pts);
        if c.len() < 2 {
            continue;
        }
        let reach = |n: usize| gr.nodes.get(n).map_or(0.0, |x| x.r) * 3.0 + 0.5;
        if e.a != e.b && gr.degree(e.b) == 1 {
            extend_end(&mut c, segs, reach(e.b));
        }
        if e.a != e.b && gr.degree(e.a) == 1 {
            c.reverse();
            extend_end(&mut c, segs, reach(e.a));
            c.reverse();
        }
        lines.push((e.a, e.b, c));
    }
    for (i, n) in gr.nodes.iter().enumerate() {
        if gr.degree(i) == 0 && n.edges.iter().all(|&e| gr.edges.get(e).is_none_or(|x| !x.alive)) {
            let r = n.r.max(grid.px);
            let (a, b) = ((n.pos.0 - r, n.pos.1), (n.pos.0 + r, n.pos.1));
            let k = ((2.0 * r / STEP_MM).ceil() as usize).clamp(1, 1000);
            let c: Vec<P> = (0..=k).map(|j| (a.0 + (b.0 - a.0) * j as f64 / k as f64, a.1)).collect();
            lines.push((next_node, next_node + 1, c));
            next_node += 2;
        }
    }
    for (a, b, c) in lines {
        let hw: Vec<f64> = c.iter().map(|&q| grid.half_width(q)).collect();
        let typical = {
            let mut v: Vec<f64> = hw.iter().copied().filter(|&h| h > 0.0).collect();
            v.sort_by(f64::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(grid.px)
        };
        let (mut left, mut right) = (Vec::with_capacity(c.len()), Vec::with_capacity(c.len()));
        let mut column = Vec::with_capacity(c.len());
        for (i, &q) in c.iter().enumerate() {
            let window = hw.get(i.saturating_sub(4)..(i + 5).min(hw.len())).unwrap_or(&[]);
            let local = window.iter().copied().fold(0.0, f64::max);
            let local = if local > 0.0 { local } else { typical };
            let hold = RAIL_HOLD * local + grid.px;
            let t = tangent(&c, i);
            let n = (-t.1, t.0);
            let dl = ray(segs, q, n).map_or(local, |d| d.min(hold));
            let dr = ray(segs, q, (-n.0, -n.1)).map_or(local, |d| d.min(hold));
            left.push((q.0 + n.0 * dl, q.1 + n.1 * dl));
            right.push((q.0 - n.0 * dr, q.1 - n.1 * dr));
            column.push(mode == LetterMode::Satin && dl + dr >= p.min_column);
        }
        out.push(Stroke { a, b, centre: c, left, right, column });
    }
    out
}

/// What letter `rings` becomes in `mode` (satin or run; a letter too wide for satin becomes
/// fill), as columns, run lines and fill outlines: the objects Convert to Stitch Objects makes.
pub fn shape_letter(rings: &[Vec<P>], mode: LetterMode, p: &LetteringParams) -> Result<LetterShape, String> {
    let a = analyse(rings, mode, p)?;
    let mut shape = LetterShape { mode: a.mode, columns: vec![], runs: vec![], fill: vec![] };
    if a.mode == LetterMode::Fill {
        shape.fill = rings.to_vec();
        return Ok(shape);
    }
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
        let mut c = s.centre.clone();
        if !forward {
            c.reverse();
        }
        let pts = run_stitches(&c, &[], false, units(length));
        self.out.add(&pts)
    }

    /// Stroke `i` sewn as it shows: satin columns and runs, forwards or back.
    fn cover(&mut self, i: usize, forward: bool) -> Result<(), String> {
        let Some(s) = self.a.strokes.get(i) else { return Ok(()) };
        let mut ps = pieces(s);
        if !forward {
            ps.reverse();
        }
        let rp = RailsParams { spacing: self.p.spacing, max_stitch: self.p.max_stitch, compensation: self.p.compensation };
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
                let mut c = c.to_vec();
                if !forward {
                    c.reverse();
                }
                run_stitches(&c, &[], false, units(self.p.run_length))
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
            let under = self.p.underlay && self.a.mode == LetterMode::Satin && self.has_column(i);
            let walk = if self.a.mode == LetterMode::Run { self.p.run_length } else { UNDERLAY_LENGTH_MM };
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

/// Sew one letter starting near `from` and ending towards `target`.
fn sew_letter(letter: &Letter, mode: LetterMode, p: &LetteringParams, from: Option<Pt>, target: P, budget: usize) -> Result<SewnLetter, String> {
    let a = analyse(&letter.rings, mode, p)?;
    let mut sewn = SewnLetter { rgb: letter.rgb, word: letter.word, mode: a.mode, runs: vec![], columns: 0, run_lines: 0 };
    if a.mode == LetterMode::Fill {
        let fp = FillParams { max_stitch: p.max_stitch, ..p.fill };
        sewn.runs = tatami(&letter.rings, FillRule::NonZero, &fp, budget)?;
        return Ok(sewn);
    }
    let mut r = Router {
        a: &a,
        p,
        sewn: vec![false; a.strokes.len()],
        seen: vec![false; a.nodes.len()],
        target,
        out: Out { runs: vec![], budget, max: (p.max_stitch - 0.15).max(0.3), count: 0 },
        columns: 0,
        run_lines: 0,
    };
    let mut at = from.map(|q| q.to_mm());
    while r.sewn.iter().any(|s| !s) {
        // The next part starts at its node nearest the needle (closest join).
        let open: Vec<usize> = (0..a.nodes.len()).filter(|&n| !r.open_at(n).is_empty()).collect();
        let near = |n: &usize| a.nodes.get(*n).map_or(f64::INFINITY, |q| at.map_or(q.0, |m| dist(*q, m)));
        let Some(&start) = open.iter().min_by(|x, y| near(x).total_cmp(&near(y))) else { break };
        if let Some(f) = r.seen.get_mut(start) {
            *f = true;
        }
        r.out.runs.push(vec![]);
        r.visit(start, false)?;
        at = r.out.pos().map(|q| q.to_mm());
    }
    sewn.runs = r.out.runs.into_iter().filter(|run| run.len() > 1).collect();
    sewn.columns = r.columns;
    sewn.run_lines = r.run_lines;
    Ok(sewn)
}

/// The box of a letter's outline: (min x, min y, max x, max y).
fn bounds(l: &Letter) -> Option<(f64, f64, f64, f64)> {
    let mut it = l.rings.iter().flatten().filter(|q| finite(**q));
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
    let budget = budget.min(MAX_STEPS);
    let mut out: Vec<SewnLetter> = Vec::with_capacity(letters.len());
    let mut pos: Option<Pt> = None;
    let mut used = 0usize;
    for (i, l) in letters.iter().enumerate() {
        // Towards the left middle of the next letter, or this one's right.
        let target = match letters.get(i + 1).and_then(bounds) {
            Some(b) => (b.0, (b.1 + b.3) / 2.0),
            None => bounds(l).map_or((0.0, 0.0), |b| (b.2, (b.1 + b.3) / 2.0)),
        };
        let s = sew_letter(l, mode, p, pos, target, budget.saturating_sub(used))?;
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
