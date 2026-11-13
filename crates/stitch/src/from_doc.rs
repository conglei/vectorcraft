//! Stitches from a document: each path and compound path is sewn by its stitch properties
//! ([`vectorcraft_doc::StitchProps`], set on it or inherited from what holds it), field by field
//! over the document-wide [`FromDocOptions`]; an automatic object's fill becomes tatami and its
//! stroke running stitch (or satin), as those options say. Stitches are made from the outlines
//! each time, so an object moved, scaled or reshaped is sewn again at the same density. Type is
//! sewn as lettering ([`crate::lettering`]); a path of exactly two open subpaths with stitch type
//! satin is a two-rail satin column.

use std::collections::HashSet;

use vectorcraft_color::Paint;
use vectorcraft_doc::{Document, LetterTrims, Node, NodeId, NodeKind, StitchParam, StitchProps, StitchType};
use vectorcraft_geom::{AnchorKind, BezPath, FillRule, PathData, PathEl, SubPath};

use crate::fill::{FillParams, tatami};
use crate::lettering::UNDERLAY_LENGTH_MM;
use crate::plan::MM_PER_PT;
use crate::run::run_stitches;
use crate::satin::{RailsParams, SatinParams, rails_satin, resample, satin};
use crate::to_doc::{follows_trim, is_marked};
use crate::{Block, MAX_STEPS, Pt, Step, StitchPlan, TOO_MANY_STEPS, Thread};

mod sequenced;
#[cfg(test)]
mod tests_lettering;
#[cfg(test)]
mod tests_overlap;
#[cfg(test)]
mod tests_sequence;
mod text;

use sequenced::{FillSpec, Run, Unit, points_of};
pub use sequenced::{SequenceItem, Sequenced, closest_join_ways, colour_sort_order, covered_fills, outline_fraction, outline_point, sequence};
pub use text::{Lettered, lettering_of, lettering_params, letters_of, text_mode};

/// Letters closer than [`crate::NO_TRIM_GAP_MM`], in plan units, are joined by a stitch.
const NO_TRIM_GAP: f64 = crate::lettering::NO_TRIM_GAP_MM * crate::UNITS_PER_MM;

/// How far a flattened curve may stray from the true one, in millimetres.
const FLATTEN_TOLERANCE_MM: f64 = 0.05;

/// How a path becomes stitches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Running stitch along the outline, stepped at the stitch length.
    #[default]
    Run,
    /// One needle point per anchor; curves are ignored.
    Manual,
}

/// What a filled path becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillType {
    /// Tatami fill: rows of running stitch across the region.
    #[default]
    Tatami,
    /// Fills are not sewn.
    None,
}

/// How filled paths are sewn.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct FillOptions {
    pub kind: FillType,
    /// Tatami settings; its `max_stitch` is replaced by [`FromDocOptions::max_stitch`].
    pub params: FillParams,
}

/// What a stroked path becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StrokeType {
    /// Running stitch along the path (or anchor by anchor, by [`FromDocOptions::mode`]).
    #[default]
    Run,
    /// A satin column centred on the path.
    Satin,
}

/// How stroked paths are sewn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeOptions {
    pub kind: StrokeType,
    /// Satin spacing, peak to peak, in mm.
    pub spacing: f64,
    /// Satin width in mm; `None`: the stroke weight, held to [`SATIN_WIDTH_MM`].
    pub width: Option<f64>,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        Self { kind: StrokeType::Run, spacing: 0.4, width: None }
    }
}

/// The satin widths a stroke weight is held to, in mm.
pub const SATIN_WIDTH_MM: (f64, f64) = (0.5, 20.0);

/// The width of a satin column sewn along an object without a stroke (stitch type satin on a
/// filled path) when neither the object nor the options give one, in mm.
pub const DEFAULT_SATIN_WIDTH_MM: f64 = 3.0;

/// A length in mm in plan units. Stitch property lengths are checked to their ranges (at most
/// 20 mm), so they fit.
fn mm_units(mm: f64) -> i32 {
    (mm * crate::UNITS_PER_MM).round() as i32
}

/// Options for [`from_document`].
#[derive(Clone, Debug)]
pub struct FromDocOptions {
    /// How every path becomes stitches. `None` (not given): art under a layer that
    /// [`crate::to_document`] made, an opened stitch file, is taken anchor by anchor
    /// ([`Mode::Manual`]) so its needle points come back unchanged; all other art is
    /// [`Mode::Run`].
    pub mode: Option<Mode>,
    /// Running stitch length, in plan units (0.1 mm).
    pub stitch_length: i32,
    /// Runs that start within this distance (plan units) of where the last one ended are joined
    /// without a trim.
    pub min_stitch: i32,
    /// Stitch only these objects and what they contain; `None`: the whole document.
    pub only: Option<Vec<NodeId>>,
    /// The longest stitch, in plan units: fills and satin columns split longer ones.
    pub max_stitch: i32,
    /// How filled paths are sewn (not on an opened stitch file's layer).
    pub fill: FillOptions,
    /// How stroked paths are sewn (satin not on an opened stitch file's layer).
    pub stroke: StrokeOptions,
}

impl Default for FromDocOptions {
    fn default() -> Self {
        Self {
            mode: None,
            stitch_length: 25,
            min_stitch: 3,
            only: None,
            max_stitch: 121,
            fill: FillOptions::default(),
            stroke: StrokeOptions::default(),
        }
    }
}

/// A plan made from a document, with what the user should be told about it.
#[derive(Debug)]
pub struct Planned {
    pub plan: StitchPlan,
    pub warnings: Vec<String>,
    /// Runs sewn from art under a layer that [`crate::to_document`] made (an opened stitch file).
    pub marked_runs: usize,
    /// Runs sewn from all other art.
    pub unmarked_runs: usize,
    /// The sewn objects in sewing order: what each tag of [`Planned::tags`] stands for.
    pub owners: Vec<NodeId>,
    /// For each step of the plan (`tags[block][step]`), the index into [`Planned::owners`] of the
    /// object it sews (a jump and trim leading to an object are its own). [`crate::prepare_tagged`]
    /// carries them through preparing the plan for a machine.
    pub tags: Vec<Vec<u32>>,
}

/// The stitch plan for `doc`'s visible filled and stroked paths and compound paths, bottom to
/// top. A fill is sewn first (tatami), then the outline (running stitch or satin), each subpath
/// one run; consecutive runs of one thread colour share a block. Template and non-printing
/// layers, guides and clipping paths are not sewn; art inside a clip is sewn in full. On a layer
/// an opened stitch file made, fills and satin are not used: its art keeps its needle points.
pub fn from_document(doc: &Document, o: &FromDocOptions) -> Result<Planned, String> {
    let mut w = Walk::new(o);
    // `doc.layers` is stored bottom first, the order the renderer paints them in.
    for layer in &doc.layers {
        w.node(layer, Under::default())?;
    }
    // Sewn in the design's order (Embroidery › Sequence), each object entered and left where its
    // points say; without them, in document order as walked.
    w.sew_units(doc)?;
    if w.blocks.is_empty() {
        return Err(NOTHING.into());
    }
    let mut warnings = vec![];
    if w.filled > 0 {
        warnings.push(format!("{} filled objects without a stroke were not stitched", w.filled));
    }
    if w.other > 0 {
        warnings.push(format!("{} objects of other kinds (type, images, symbols) were not stitched", w.other));
    }
    if w.patterned {
        warnings.push("a stroke with a gradient or pattern was stitched in black".into());
    }
    if w.patterned_fill {
        warnings.push("a fill with a gradient or pattern was stitched in black".into());
    }
    if w.held_width {
        warnings.push(format!("a satin width was held to {}–{} mm", SATIN_WIDTH_MM.0, SATIN_WIDTH_MM.1));
    }
    if w.clipped {
        warnings.push("clipped art was stitched in full: the clip was ignored".into());
    }
    let plan = StitchPlan { name: String::new(), blocks: w.blocks };
    plan.check()?;
    Ok(Planned { plan, warnings, marked_runs: w.marked_runs, unmarked_runs: w.unmarked_runs, owners: w.owners, tags: w.tags })
}

/// The error for a document with nothing to sew.
const NOTHING: &str = "nothing to stitch: the document has no stroked or filled paths";

struct Walk<'o> {
    o: &'o FromDocOptions,
    /// [`FromDocOptions::only`] as a set.
    only: Option<HashSet<NodeId>>,
    blocks: Vec<Block>,
    /// For each step of `blocks`, the object it sews: an index into `owners`.
    tags: Vec<Vec<u32>>,
    /// The objects sewn, in sewing order ([`Planned::owners`]).
    owners: Vec<NodeId>,
    /// Where the needle is: the last stitch made.
    pos: Option<Pt>,
    steps: usize,
    filled: usize,
    other: usize,
    patterned: bool,
    patterned_fill: bool,
    /// A stroke weight was outside [`SATIN_WIDTH_MM`].
    held_width: bool,
    /// Art inside a clip was sewn.
    clipped: bool,
    marked_runs: usize,
    unmarked_runs: usize,
    /// The objects walked so far, each with its runs, sewn once the walk is done.
    units: Vec<Unit>,
    /// Stitch types sewn so far: each fill or outline of an object is one.
    part: usize,
}

/// How a run follows the one before it in the same thread colour.
#[derive(Clone, Copy, Debug)]
enum Join {
    /// Art drawn by hand: a trim and a jump, unless the runs are within `min_stitch`.
    Drawn,
    /// A run of an opened stitch file: a jump, preceded by a trim only where `trim` says the
    /// file had one.
    Opened { trim: bool },
    /// The next letter (or part of a letter) of lettering: a trim and a jump where `trim` says
    /// (between words, by default), else a stitch to it when it is closer than
    /// [`crate::NO_TRIM_GAP_MM`] and a jump when further.
    Letter { trim: bool },
}

impl Join {
    /// Whether the join counts as a trim before a run that starts `d` plan units from the needle,
    /// given the join distance `min_stitch` (for choosing how an object is turned).
    pub(super) fn trims(self, d: f64, min_stitch: i32) -> bool {
        d > f64::from(min_stitch)
            && match self {
                Join::Letter { trim } => trim,
                Join::Drawn | Join::Opened { .. } => true,
            }
    }
}

/// What a node lies under.
#[derive(Clone, Copy, Default)]
struct Under {
    /// An ancestor is in [`FromDocOptions::only`].
    listed: bool,
    /// An ancestor clips it.
    clipped: bool,
    /// A layer an opened stitch file made holds it.
    marked: bool,
    /// The stitch properties it inherits from what holds it.
    props: StitchProps,
}

/// How an outline is sewn.
#[derive(Clone, Copy)]
enum Outline {
    /// Running stitch (or, by [`Mode::Manual`], anchor by anchor), at this stitch length in plan
    /// units.
    Run(Mode, i32),
    Satin(SatinParams),
}

impl<'o> Walk<'o> {
    fn new(o: &'o FromDocOptions) -> Walk<'o> {
        let only = o.only.as_ref().map(|ids| ids.iter().copied().collect());
        Walk {
            o,
            only,
            blocks: vec![],
            tags: vec![],
            owners: vec![],
            pos: None,
            steps: 0,
            filled: 0,
            other: 0,
            patterned: false,
            patterned_fill: false,
            held_width: false,
            clipped: false,
            marked_runs: 0,
            unmarked_runs: 0,
            units: vec![],
            part: 0,
        }
    }

    /// Stitch `n` and what it holds, given what it lies under. Recursion is bounded by the
    /// document's own nesting limit.
    fn node(&mut self, n: &Node, up: Under) -> Result<(), String> {
        if !n.visible {
            return Ok(());
        }
        // Template and non-printing layers don't print, so they aren't sewn either.
        if let NodeKind::Layer { template, printable, .. } = n.kind
            && (template || !printable)
        {
            return Ok(());
        }
        let up = Under {
            listed: up.listed || self.only.as_ref().is_none_or(|ids| ids.contains(&n.id)),
            clipped: up.clipped,
            marked: up.marked || is_marked(n),
            props: n.stitch_props().map_or(up.props, |p| p.inherit(&up.props)),
        };
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
                // A clip group's or clipping layer's first child is its clipping path.
                let clips = n.clips();
                for c in children.iter().skip(usize::from(clips)) {
                    self.node(c, Under { clipped: up.clipped || clips, ..up })?;
                }
            }
            _ if !up.listed => {}
            // Guides and clipping paths don't print, so they aren't sewn either.
            NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                // The object's own entry and exit points (never inherited; not on an opened
                // stitch file's layer, whose needle points are kept).
                let own = n.stitch_props().filter(|_| !up.marked);
                let pin = points_of(n, (own.and_then(|p| p.entry_at()), own.and_then(|p| p.exit_at())));
                let prefer = points_of(n, (own.and_then(|p| p.join_entry_at()), own.and_then(|p| p.join_exit_at())));
                let mut unit = Unit::new(n.id, pin, prefer, false);
                unit.overlap = own.and_then(StitchProps::overlap_mm);
                unit.route = own.and_then(StitchProps::route_mm);
                self.units.push(unit);
                self.shape(n, up)?;
                if self.units.last().is_some_and(|u| u.runs.is_empty()) {
                    self.units.pop();
                }
            }
            NodeKind::Text(_) if !up.marked => {
                // Lettering is sewn in its own order, letter by letter: never turned round.
                let mut unit = Unit::new(n.id, (None, None), (None, None), true);
                unit.route = n.stitch_props().and_then(StitchProps::route_mm);
                self.units.push(unit);
                self.lettering(n, up)?;
                if self.units.last().is_some_and(|u| u.runs.is_empty()) {
                    self.units.pop();
                }
            }
            _ => self.other += 1,
        }
        Ok(())
    }

    /// Sew a path or compound path: by its stitch type ([`Under::props`], what it set and
    /// inherited), or, automatic, as the document's options say. Art on an opened stitch file's
    /// layer keeps its needle points: stitch properties are not used there.
    fn shape(&mut self, n: &Node, up: Under) -> Result<(), String> {
        let props = if up.marked { StitchProps::default() } else { up.props };
        let stroke = n.appearance.stroke().filter(|s| s.visible && !s.paint.is_none());
        let painted_fill = n.appearance.fill().filter(|f| f.visible && !f.paint.is_none());
        let max_mm = f64::from(self.o.max_stitch) / crate::UNITS_PER_MM;
        let mut subpaths: Vec<&SubPath> = vec![];
        let run_length = |w: &Walk| props.number(StitchParam::RunLength).map_or(w.o.stitch_length, mm_units);
        match props.kind {
            StitchType::None => Ok(()),
            // Lettering is for type: a path set (or inheriting) it sews as automatic.
            StitchType::Auto | StitchType::Lettering => {
                let sew_fill = !up.marked && self.o.fill.kind == FillType::Tatami;
                let fill = painted_fill.filter(|_| sew_fill);
                if stroke.is_none() && fill.is_none() {
                    if !n.appearance.fill_paint().is_none() {
                        self.filled += 1;
                    }
                    return Ok(());
                }
                collect_subpaths(n, &mut subpaths);
                if let Some(f) = fill {
                    let rgb = thread(&f.paint, &mut self.patterned_fill);
                    self.sew_fill(n, &subpaths, rgb, &self.fill_params(&props, max_mm), up)?;
                }
                let Some(stroke) = stroke else { return Ok(()) };
                let rgb = thread(&stroke.paint, &mut self.patterned);
                let outline = if self.o.stroke.kind == StrokeType::Satin && !up.marked {
                    Outline::Satin(self.satin_params(&props, Some(stroke.width), max_mm))
                } else {
                    let mode = self.o.mode.unwrap_or(if up.marked { Mode::Manual } else { Mode::Run });
                    Outline::Run(mode, run_length(self))
                };
                // Only the first run of a path can follow the trim a file held.
                self.sew_outline(&subpaths, rgb, outline, up, up.marked && follows_trim(n))
            }
            StitchType::Tatami => {
                // The fill colour, else the stroke's: the region the outline encloses.
                let Some(rgb) = self.colour(painted_fill.map(|f| &f.paint), stroke.map(|s| &s.paint), true) else { return Ok(()) };
                collect_subpaths(n, &mut subpaths);
                self.sew_fill(n, &subpaths, rgb, &self.fill_params(&props, max_mm), up)
            }
            StitchType::Run | StitchType::Satin | StitchType::Manual => {
                // The stroke colour, else the fill's: one outline, whatever the object paints.
                let Some(rgb) = self.colour(painted_fill.map(|f| &f.paint), stroke.map(|s| &s.paint), false) else { return Ok(()) };
                collect_subpaths(n, &mut subpaths);
                if props.kind == StitchType::Satin
                    && let Some((a, b)) = two_rails(&subpaths)
                {
                    return self.sew_rails(a, b, rgb, &props, max_mm, up);
                }
                let outline = match props.kind {
                    StitchType::Satin => Outline::Satin(self.satin_params(&props, stroke.map(|s| s.width), max_mm)),
                    StitchType::Manual => Outline::Run(Mode::Manual, run_length(self)),
                    _ => Outline::Run(Mode::Run, run_length(self)),
                };
                self.sew_outline(&subpaths, rgb, outline, up, false)
            }
        }
    }

    /// The thread colour of the fill (`fill`, when painted) or the stroke (`stroke`), the fill's
    /// first when `fill_first`, else the stroke's; `None` when neither is painted. A gradient or
    /// pattern is sewn in black and noted.
    fn colour(&mut self, fill: Option<&Paint>, stroke: Option<&Paint>, fill_first: bool) -> Option<[u8; 3]> {
        match (fill, stroke) {
            (Some(f), _) if fill_first => Some(thread(f, &mut self.patterned_fill)),
            (_, Some(s)) => Some(thread(s, &mut self.patterned)),
            (Some(f), None) => Some(thread(f, &mut self.patterned_fill)),
            (None, None) => None,
        }
    }

    /// The tatami settings for an object with properties `props`: each number it sets, else the
    /// document's.
    fn fill_params(&self, props: &StitchProps, max_mm: f64) -> FillParams {
        let d = self.o.fill.params;
        let n = |p: StitchParam, default: f64| props.number(p).unwrap_or(default);
        FillParams {
            angle: n(StitchParam::FillAngle, d.angle),
            spacing: n(StitchParam::FillSpacing, d.spacing),
            stitch_length: n(StitchParam::FillStitchLength, d.stitch_length),
            stagger: n(StitchParam::Stagger, d.stagger),
            max_stitch: max_mm,
        }
    }

    /// The satin settings for an object with properties `props` and stroke weight `weight_pt`
    /// (`None`: no stroke): its width, else the document's, else the stroke weight held to
    /// [`SATIN_WIDTH_MM`] (noted when held), else [`DEFAULT_SATIN_WIDTH_MM`].
    fn satin_params(&mut self, props: &StitchProps, weight_pt: Option<f64>, max_mm: f64) -> SatinParams {
        let (lo, hi) = SATIN_WIDTH_MM;
        let width = match (props.number(StitchParam::SatinWidth).or(self.o.stroke.width), weight_pt) {
            (Some(w), _) => w,
            (None, Some(weight)) => {
                let w = weight * MM_PER_PT;
                let held = if w.is_finite() { w.clamp(lo, hi) } else { lo };
                self.held_width |= held != w;
                held
            }
            (None, None) => DEFAULT_SATIN_WIDTH_MM,
        };
        let spacing = props.number(StitchParam::SatinSpacing).unwrap_or(self.o.stroke.spacing);
        SatinParams { width, spacing, max_stitch: max_mm }
    }

    /// Sew the region `subpaths` of `n` enclose as tatami, by `n`'s fill rule, in thread `rgb`.
    fn sew_fill(&mut self, n: &Node, subpaths: &[&SubPath], rgb: [u8; 3], params: &FillParams, up: Under) -> Result<(), String> {
        let rule = match &n.kind {
            NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
            _ => FillRule::default(),
        };
        self.part += 1;
        let mut rings = vec![];
        for sp in subpaths {
            rings.push(flatten(sp, MAX_STEPS.saturating_sub(self.steps))?.0);
        }
        for run in tatami(&rings, rule, params, MAX_STEPS.saturating_sub(self.steps))? {
            self.count_run(&run, up);
            // Fills are never on an opened stitch file's layer.
            self.add_run(rgb, &run, Mode::Run, Join::Drawn)?;
        }
        // Kept for overlap removal and routed travel, which need the fills sewn after it.
        let path = PathData::new(subpaths.iter().map(|sp| (*sp).clone()).collect());
        let part = self.part;
        if let Some(u) = self.units.last_mut() {
            u.fill = Some(FillSpec { part, rgb, params: *params, path, rule, rings });
        }
        Ok(())
    }

    /// Sew each of `subpaths` as `outline`, in thread `rgb`; `trim`: its first run follows a trim
    /// the opened file held.
    fn sew_outline(&mut self, subpaths: &[&SubPath], rgb: [u8; 3], outline: Outline, up: Under, mut trim: bool) -> Result<(), String> {
        self.part += 1;
        for sp in subpaths {
            let budget = MAX_STEPS.saturating_sub(self.steps);
            let (run, mode) = match outline {
                Outline::Satin(p) => {
                    // A satin column is sewn like running stitch: a first stitch where the needle
                    // is would sew nothing.
                    (satin(&flatten(sp, budget)?.0, sp.closed && sp.segment_count() > 0, &p, budget)?, Mode::Run)
                }
                Outline::Run(Mode::Run, length) => (run_of(sp, length, budget)?, Mode::Run),
                Outline::Run(Mode::Manual, _) => (manual_of(sp, budget)?, Mode::Manual),
            };
            self.count_run(&run, up);
            let after = if up.marked { Join::Opened { trim } } else { Join::Drawn };
            if !run.is_empty() {
                trim = false;
            }
            self.add_run(rgb, &run, mode, after)?;
        }
        Ok(())
    }

    /// Sew rails `a` and `b` (two open subpaths) as one satin column, in thread `rgb`: with the
    /// object's underlay on, a centre run along it first and the satin back over it.
    fn sew_rails(&mut self, a: &SubPath, b: &SubPath, rgb: [u8; 3], props: &StitchProps, max_mm: f64, up: Under) -> Result<(), String> {
        self.part += 1;
        let budget = MAX_STEPS.saturating_sub(self.steps);
        let (left, right) = (flatten(a, budget)?.0, flatten(b, budget)?.0);
        let n = left.len().max(right.len());
        if n < 2 || n > budget {
            return Ok(());
        }
        let (mut left, mut right) = (resample(&left, n), resample(&right, n));
        // Rails drawn the opposite way round are turned to run together. Their ends and middles
        // and quarters are compared: a closed column (a ring) starts and ends at one place, so only
        // its quarters tell the two ways apart.
        let pairing = |turned: bool| -> f64 {
            [0, n / 4, n / 2, 3 * n / 4, n - 1]
                .iter()
                .filter_map(|&i| Some((left.get(i)?, right.get(if turned { n - 1 - i } else { i })?)))
                .map(|(p, q)| (p.0 - q.0).hypot(p.1 - q.1))
                .sum()
        };
        if pairing(true) < pairing(false) {
            right.reverse();
        }
        let p = RailsParams {
            spacing: props.number(StitchParam::SatinSpacing).unwrap_or(self.o.stroke.spacing),
            max_stitch: max_mm,
            compensation: props.number(StitchParam::Compensation).unwrap_or(0.0),
        };
        let mut run: Vec<Pt> = vec![];
        if props.underlay == Some(true) {
            let centre: Vec<(f64, f64)> = left.iter().zip(&right).map(|(l, r)| ((l.0 + r.0) / 2.0, (l.1 + r.1) / 2.0)).collect();
            run = run_stitches(&centre, &[], false, mm_units(UNDERLAY_LENGTH_MM));
            left.reverse();
            right.reverse();
            // Turned round, the underlay would be sewn over the satin: the column is sewn forwards.
            if let Some(u) = self.units.last_mut() {
                u.fixed = true;
            }
        }
        for q in rails_satin(&left, &right, &p, budget)? {
            if run.last() != Some(&q) {
                run.push(q);
            }
        }
        self.count_run(&run, up);
        self.add_run(rgb, &run, Mode::Run, Join::Drawn)
    }

    /// Sew text node `n` as lettering, in its letters' threads.
    fn lettering(&mut self, n: &Node, up: Under) -> Result<(), String> {
        let props = up.props;
        if text::text_mode(props.kind).is_none() {
            if props.kind != StitchType::None {
                self.other += 1;
            }
            return Ok(());
        }
        let budget = MAX_STEPS.saturating_sub(self.steps);
        let Some(done) = text::lettering_of(n, &props, self.o, budget)? else { return Ok(()) };
        self.part += 1;
        let mut word: Option<usize> = None;
        for letter in done.letters {
            for (k, run) in letter.runs.iter().enumerate() {
                let join = match word {
                    None => Join::Drawn,
                    Some(_) if k > 0 => Join::Letter { trim: false },
                    Some(w) => Join::Letter {
                        trim: match done.params.trims {
                            LetterTrims::Letter => true,
                            LetterTrims::Word => w != letter.word,
                            LetterTrims::None => false,
                        },
                    },
                };
                self.count_run(run, up);
                self.add_run(letter.rgb, run, Mode::Run, join)?;
                word = Some(letter.word);
            }
        }
        Ok(())
    }

    /// Count `run` as marked or unmarked art, and note art inside a clip.
    fn count_run(&mut self, run: &[Pt], up: Under) {
        if run.is_empty() {
            return;
        }
        self.clipped |= up.clipped;
        if up.marked {
            self.marked_runs += 1;
        } else {
            self.unmarked_runs += 1;
        }
    }

    /// Keep `run` for the object being walked (sewn by [`Walk::emit_run`] once the sewing order
    /// is known). The step limit counts it now, so a design past it stops early.
    fn add_run(&mut self, rgb: [u8; 3], run: &[Pt], mode: Mode, join: Join) -> Result<(), String> {
        if run.is_empty() {
            return Ok(());
        }
        self.steps = self.steps.saturating_add(run.len());
        if self.steps > MAX_STEPS {
            return Err(TOO_MANY_STEPS.to_string());
        }
        let part = self.part;
        if let Some(u) = self.units.last_mut() {
            u.runs.push(Run { rgb, pts: run.to_vec(), mode, join, part });
        }
        Ok(())
    }

    /// Sew `run` in thread `rgb`: a new block at a colour change (which starts with a jump to the
    /// run, except at the very start), otherwise a trim and a jump unless the run starts within
    /// `min_stitch` of the needle. A run of an opened stitch file ([`Join::Opened`]) is trimmed
    /// only where the file trimmed.
    fn emit_run(&mut self, rgb: [u8; 3], run: &[Pt], mode: Mode, join: Join) -> Result<(), String> {
        let Some(&start) = run.first() else { return Ok(()) };
        let mut skip_first = false;
        let mut lead: Vec<Step> = vec![];
        match self.blocks.last() {
            Some(b) if b.thread.rgb == rgb => {
                let near = self.pos.is_some_and(|p| p.dist(start) <= f64::from(self.o.min_stitch));
                match join {
                    Join::Opened { trim: true } => {
                        lead.push(Step::Trim);
                        if self.pos != Some(start) {
                            lead.push(Step::Jump(start));
                        }
                    }
                    Join::Opened { trim: false } if !near => lead.push(Step::Jump(start)),
                    _ if near => {
                        // In run mode a first stitch where the needle already is would sew nothing.
                        skip_first = mode == Mode::Run && self.pos == Some(start);
                    }
                    // Letters of a word: a stitch across a small gap, else a jump; no trim.
                    Join::Letter { trim: false } => {
                        if self.pos.is_none_or(|p| p.dist(start) > NO_TRIM_GAP) {
                            lead.push(Step::Jump(start));
                        }
                    }
                    _ => lead = vec![Step::Trim, Step::Jump(start)],
                }
            }
            last => {
                if last.is_some() {
                    lead.push(Step::Jump(start));
                }
                self.blocks.push(Block { thread: Thread { rgb, name: None, code: None, brand: None }, steps: vec![] });
                self.tags.push(vec![]);
            }
        }
        let added = lead.len() + run.len();
        self.steps = self.steps.saturating_add(added);
        if self.steps > MAX_STEPS {
            return Err(TOO_MANY_STEPS.to_string());
        }
        let Some(block) = self.blocks.last_mut() else { return Ok(()) };
        block.steps.extend(lead);
        block.steps.extend(run.iter().skip(usize::from(skip_first)).map(|p| Step::Stitch(*p)));
        // The object being sewn is the last owner.
        let owner = u32::try_from(self.owners.len().saturating_sub(1)).unwrap_or(u32::MAX);
        if let Some(tags) = self.tags.last_mut() {
            tags.resize(block.steps.len(), owner);
        }
        self.pos = run.last().copied();
        Ok(())
    }
}

/// The thread colour of `paint`: its solid colour, or black for a gradient or pattern (noted in
/// `patterned`).
fn thread(paint: &Paint, patterned: &mut bool) -> [u8; 3] {
    match paint {
        Paint::Solid { color, .. } => {
            let [r, g, b, _] = color.to_rgba8(1.0);
            [r, g, b]
        }
        _ => {
            *patterned = true;
            [0, 0, 0]
        }
    }
}

/// The two rails of a satin column drawn as rails: exactly two open subpaths, each with a
/// segment.
pub(crate) fn two_rails<'a>(subpaths: &[&'a SubPath]) -> Option<(&'a SubPath, &'a SubPath)> {
    match subpaths {
        [a, b] if !a.closed && !b.closed && a.segment_count() > 0 && b.segment_count() > 0 => Some((a, b)),
        _ => None,
    }
}

/// The subpaths of a path, or of a compound path's member paths (in their order).
pub(crate) fn collect_subpaths<'n>(n: &'n Node, out: &mut Vec<&'n SubPath>) {
    match &n.kind {
        NodeKind::Path { path, .. } => out.extend(path.subpaths.iter()),
        NodeKind::Compound { children, .. } => {
            for c in children {
                collect_subpaths(c, out);
            }
        }
        _ => {}
    }
}

/// Flattened points in millimetres and a corner flag for each.
type Flat = (Vec<(f64, f64)>, Vec<bool>);

const TOO_FAR: &str = "the art lies too far from the origin to stitch";

/// A document point in millimetres, if it fits a plan point.
fn mm(p: vectorcraft_geom::Point) -> Result<(f64, f64), String> {
    let q = (p.x * MM_PER_PT, p.y * MM_PER_PT);
    match Pt::from_mm(q.0, q.1) {
        Some(_) => Ok(q),
        None => Err(TOO_FAR.into()),
    }
}

/// Running stitch along `sp`, flattened, with its corner anchors kept as corners. More than
/// `budget` flattened points is an error, found before they are all made.
fn run_of(sp: &SubPath, stitch_length: i32, budget: usize) -> Result<Vec<Pt>, String> {
    let (points, corners) = flatten(sp, budget)?;
    Ok(run_stitches(&points, &corners, sp.closed && sp.segment_count() > 0, stitch_length))
}

/// `sp` flattened to millimetres, with a corner flag per point. A closed subpath's first point is
/// not repeated at the end. More than `budget` points is an error, found before they are all made.
fn flatten(sp: &SubPath, budget: usize) -> Result<Flat, String> {
    let Some(first) = sp.anchors.first() else { return Ok((vec![], vec![])) };
    let mut points = vec![mm(first.p)?];
    let mut corners = vec![first.kind == AnchorKind::Corner];
    let tolerance = FLATTEN_TOLERANCE_MM / MM_PER_PT;
    let count = sp.segment_count();
    for i in 0..count {
        let Some(end) = sp.anchors.get((i + 1) % sp.anchors.len().max(1)) else { break };
        let c = sp.segment(i);
        if points.len() >= budget {
            return Err(TOO_MANY_STEPS.into());
        }
        if sp.segment_is_line(i) {
            points.push(mm(c.p3)?);
            corners.push(end.kind == AnchorKind::Corner);
            continue;
        }
        let mut bp = BezPath::new();
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
        // Room for this curve's points; the flattening stops adding them past it.
        let room = budget.saturating_sub(points.len());
        let mut flat = vec![];
        let mut over = false;
        vectorcraft_geom::flatten(bp.elements().iter().copied(), tolerance, |el| {
            if let PathEl::LineTo(p) = el {
                if flat.len() < room {
                    flat.push(p);
                } else {
                    over = true;
                }
            }
        });
        if over {
            return Err(TOO_MANY_STEPS.into());
        }
        // The curve's own end is the anchor, whatever the flattening ended on.
        flat.pop();
        for p in flat {
            points.push(mm(p)?);
            corners.push(false);
        }
        points.push(mm(c.p3)?);
        corners.push(end.kind == AnchorKind::Corner);
    }
    if sp.closed && count > 0 {
        // The closing segment ends back on the first anchor, which callers close again.
        points.pop();
        corners.pop();
    }
    Ok((points, corners))
}

/// A subpath flattened to millimetres, and whether it is closed.
pub type FlatLine = (Vec<(f64, f64)>, bool);

/// The subpaths of path or compound path `n`, flattened to millimetres, each with whether it is
/// closed (auto-digitizing reads shapes this way). More than `budget` points is an error.
pub fn outline_mm(n: &Node, budget: usize) -> Result<Vec<FlatLine>, String> {
    let mut sps = vec![];
    collect_subpaths(n, &mut sps);
    let mut out = Vec::with_capacity(sps.len());
    let mut used = 0usize;
    for sp in sps {
        let (pts, _) = flatten(sp, budget.saturating_sub(used))?;
        used = used.saturating_add(pts.len());
        if pts.len() >= 2 {
            out.push((pts, sp.closed && sp.segment_count() > 0));
        }
    }
    Ok(out)
}

/// One needle point per anchor; a closed subpath returns to its first anchor. More than
/// `budget` points is an error.
fn manual_of(sp: &SubPath, budget: usize) -> Result<Vec<Pt>, String> {
    if sp.anchors.len().saturating_add(usize::from(sp.closed)) > budget {
        return Err(TOO_MANY_STEPS.into());
    }
    let mut out = Vec::with_capacity(sp.anchors.len().saturating_add(1));
    for a in &sp.anchors {
        out.push(Pt::from_pt(a.p.x, a.p.y).ok_or(TOO_FAR)?);
    }
    if sp.closed
        && let Some(&first) = out.first()
    {
        out.push(first);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
    use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId};
    use vectorcraft_geom::{Anchor, PathData, Point, SubPath};

    use super::*;
    use crate::{PT_PER_MM, Pt, Step};

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn mm(x: f64, y: f64) -> Point {
        Point::new(x * PT_PER_MM, y * PT_PER_MM)
    }

    fn square_mm(side: f64) -> PathData {
        PathData::single(SubPath::polyline(&[mm(0.0, 0.0), mm(side, 0.0), mm(side, side), mm(0.0, side)], true))
    }

    fn line_mm(x0: f64, y0: f64, x1: f64, y1: f64) -> PathData {
        PathData::single(SubPath::polyline(&[mm(x0, y0), mm(x1, y1)], false))
    }

    fn single_anchor() -> PathData {
        PathData::single(SubPath::new(vec![Anchor::corner(mm(1.0, 1.0))], false))
    }

    fn stroked(path: PathData, rgb: [u8; 3]) -> (PathData, Appearance) {
        (path, Appearance::basic(Paint::None, Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])), 1.0))
    }

    /// A document whose one layer holds `art` as paths, bottom first.
    fn doc_with(art: Vec<(PathData, Appearance)>) -> Document {
        let mut doc = Document::new(500.0, 500.0);
        let mut layer = Node::layer(doc.alloc_id(), "Art", LayerColor::Preset(0));
        let children: Vec<Arc<Node>> = art.into_iter().map(|(p, a)| Arc::new(Node::path(doc.alloc_id(), p, a))).collect();
        if let Some(c) = layer.children_mut() {
            *c = children;
        }
        doc.layers = vec![Arc::new(layer)];
        doc
    }

    /// A filled square with no stroke, a hidden stroked line, and a visible stroked line inside a
    /// group.
    fn mixed_doc() -> Document {
        let mut doc = Document::new(500.0, 500.0);
        let filled = Node::path(doc.alloc_id(), square_mm(10.0), Appearance::basic(Paint::solid(Color::rgb8(0, 255, 0)), Paint::None, 0.0));
        let (p, a) = stroked(line_mm(0.0, 20.0, 10.0, 20.0), BLUE);
        let mut hidden = Node::path(doc.alloc_id(), p, a);
        hidden.visible = false;
        let (p, a) = stroked(line_mm(0.0, 40.0, 10.0, 40.0), RED);
        let visible = Node::path(doc.alloc_id(), p, a);
        let group = Node::group(doc.alloc_id(), vec![Arc::new(hidden), Arc::new(visible)]);
        let mut layer = Node::layer(doc.alloc_id(), "Art", LayerColor::Preset(0));
        if let Some(c) = layer.children_mut() {
            *c = vec![Arc::new(filled), Arc::new(group)];
        }
        doc.layers = vec![Arc::new(layer)];
        doc
    }

    #[test]
    fn closed_path_returns_to_its_start() {
        let plan = from_document(&doc_with(vec![stroked(square_mm(10.0), RED)]), &FromDocOptions::default()).unwrap().plan;
        let pts: Vec<Pt> = plan.points().collect();
        assert_eq!(pts.first(), pts.last());
        assert_eq!(pts.len(), 17); // 4 sides × 4 steps of 2.5 mm, plus the start
    }

    #[test]
    fn colours_make_blocks_and_objects_are_trimmed_apart() {
        let d = doc_with(vec![
            stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED),
            stroked(line_mm(0.0, 20.0, 10.0, 20.0), RED),
            stroked(line_mm(0.0, 40.0, 10.0, 40.0), BLUE),
        ]);
        let plan = from_document(&d, &FromDocOptions::default()).unwrap().plan;
        assert_eq!(plan.blocks.len(), 2);
        assert_eq!(plan.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1);
        assert_eq!(plan.blocks[0].thread.rgb, [255, 0, 0]);
        // The first block starts with its first stitch; the second, at a colour change, with a
        // jump to its first point and no trim.
        assert_eq!(plan.blocks[0].steps.first(), Some(&Step::Stitch(Pt { x: 0, y: 0 })));
        assert_eq!(plan.blocks[1].steps.first(), Some(&Step::Jump(Pt { x: 0, y: 400 })));
        assert!(!plan.blocks[1].steps.contains(&Step::Trim));
    }

    #[test]
    fn fills_hidden_art_and_other_kinds_are_skipped_with_warnings() {
        // a filled square with no stroke, a hidden stroked line, a visible stroked line
        let no_fills = FromDocOptions { fill: FillOptions { kind: FillType::None, ..Default::default() }, ..Default::default() };
        let out = from_document(&mixed_doc(), &no_fills).unwrap();
        assert_eq!(out.plan.blocks.len(), 1);
        assert_eq!(out.warnings, vec!["1 filled objects without a stroke were not stitched".to_string()]);
        // By default the fill is sewn, in its own colour, before the line.
        let out = from_document(&mixed_doc(), &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>(), vec![[0, 255, 0], RED]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    fn filled_and_stroked(path: PathData, fill: [u8; 3], stroke: [u8; 3], weight_pt: f64) -> (PathData, Appearance) {
        let c = |rgb: [u8; 3]| Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2]));
        (path, Appearance::basic(c(fill), c(stroke), weight_pt))
    }

    #[test]
    fn a_filled_and_stroked_path_is_filled_first_then_outlined() {
        let d = doc_with(vec![filled_and_stroked(square_mm(10.0), BLUE, RED, 1.0)]);
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>(), vec![BLUE, RED]);
        // The fill: rows 0.4 mm apart across 10 mm.
        let fill: Vec<Pt> = out.plan.blocks[0].steps.iter().filter_map(|s| if let Step::Stitch(p) = s { Some(*p) } else { None }).collect();
        assert!(fill.iter().all(|p| (0..=100).contains(&p.x) && (0..=100).contains(&p.y)), "{fill:?}");
        assert!(fill.len() > 25 * 4, "{}", fill.len());
        // The outline: running stitch, as before.
        assert_eq!(out.plan.blocks[1].steps.iter().filter(|s| matches!(s, Step::Stitch(_))).count(), 17);
        assert_eq!(out.unmarked_runs, 2);
    }

    #[test]
    fn a_satin_stroke_takes_the_stroke_weight_as_its_width() {
        let d = doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]);
        let satin =
            |width: Option<f64>| FromDocOptions { stroke: StrokeOptions { kind: StrokeType::Satin, spacing: 0.4, width }, ..Default::default() };
        // A 4 mm stroke weight: rails at y = +-2 mm.
        let mut heavy = d.clone();
        let mut layer = (*heavy.layers[0]).clone();
        let path = layer.children().unwrap()[0].clone();
        let mut path = (*path).clone();
        path.appearance.stroke_mut().unwrap().width = 4.0 * PT_PER_MM;
        *layer.children_mut().unwrap() = vec![Arc::new(path)];
        heavy.layers = vec![Arc::new(layer)];
        let out = from_document(&heavy, &satin(None)).unwrap();
        let ys: Vec<i32> = out.plan.points().map(|p| p.y).collect();
        assert_eq!(ys.iter().min(), Some(&-20));
        assert_eq!(ys.iter().max(), Some(&20));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        // A given width wins; a hairline stroke weight is held to the narrowest satin, with a
        // warning.
        let out = from_document(&d, &satin(Some(2.0))).unwrap();
        assert_eq!(out.plan.points().map(|p| p.y).max(), Some(10));
        let out = from_document(&d, &satin(None)).unwrap();
        assert_eq!(out.plan.points().map(|p| p.y).max(), Some(3));
        assert_eq!(out.warnings, vec!["a satin width was held to 0.5–20 mm".to_string()]);
    }

    #[test]
    fn an_opened_stitch_files_layer_keeps_its_needle_points() {
        let d = doc_with(vec![filled_and_stroked(square_mm(10.0), BLUE, RED, 1.0)]);
        let mut marked = d.clone();
        let plan = from_document(
            &d,
            &FromDocOptions { fill: FillOptions { kind: FillType::None, ..Default::default() }, mode: Some(Mode::Manual), ..Default::default() },
        )
        .unwrap()
        .plan;
        let opened = crate::to_document(&plan);
        marked.layers = opened.layers.clone();
        // Fill and satin options do not reach art on the opened file's layer.
        let o = FromDocOptions { stroke: StrokeOptions { kind: StrokeType::Satin, ..Default::default() }, ..Default::default() };
        let out = from_document(&marked, &o).unwrap();
        assert_eq!(out.plan.blocks.len(), 1);
        assert_eq!(out.marked_runs, 1);
        assert_eq!(out.plan.points().count(), 5);
    }

    #[test]
    fn a_gradient_fill_is_sewn_in_black_with_a_warning() {
        let mut a = Appearance::basic(Paint::None, Paint::None, 1.0);
        a.set_fill(Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))));
        let out = from_document(&doc_with(vec![(square_mm(5.0), a)]), &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks[0].thread.rgb, [0, 0, 0]);
        assert_eq!(out.warnings, vec!["a fill with a gradient or pattern was stitched in black".to_string()]);
    }

    #[test]
    fn degenerate_paths_add_nothing() {
        let d = doc_with(vec![stroked(single_anchor(), RED), stroked(line_mm(5.0, 5.0, 5.0, 5.0), RED)]);
        assert_eq!(from_document(&d, &FromDocOptions::default()).unwrap_err(), NOTHING);
    }

    #[test]
    fn manual_mode_uses_the_anchors() {
        let o = FromDocOptions { mode: Some(Mode::Manual), ..Default::default() };
        let plan = from_document(&doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]), &o).unwrap().plan;
        assert_eq!(plan.blocks[0].steps, vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 100, y: 0 })]);
    }

    #[test]
    fn only_the_listed_objects_and_their_contents_are_stitched() {
        let d = doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED), stroked(line_mm(0.0, 20.0, 10.0, 20.0), BLUE)]);
        let second = d.layers[0].children().unwrap()[1].id;
        let plan = from_document(&d, &FromDocOptions { only: Some(vec![second]), ..Default::default() }).unwrap().plan;
        assert_eq!(plan.blocks.len(), 1);
        assert_eq!(plan.blocks[0].thread.rgb, BLUE);
        let layer = d.layers[0].id;
        let all = from_document(&d, &FromDocOptions { only: Some(vec![layer]), ..Default::default() }).unwrap().plan;
        assert_eq!(all.blocks.len(), 2);
        let none = from_document(&d, &FromDocOptions { only: Some(vec![NodeId(9999)]), ..Default::default() });
        assert!(none.is_err());
    }

    #[test]
    fn gradient_and_pattern_strokes_are_stitched_in_black_with_one_warning() {
        let painted = |doc: &mut Document, y: f64, paint: Paint| {
            let mut a = Appearance::basic(Paint::None, Paint::None, 1.0);
            if let Some(s) = a.stroke_mut() {
                s.paint = paint;
            }
            Arc::new(Node::path(doc.alloc_id(), line_mm(0.0, y, 10.0, y), a))
        };
        let mut d = doc_with(vec![]);
        let pattern = Paint::Pattern { pattern: "dots".into(), xf: Default::default() };
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let children = vec![painted(&mut d, 0.0, gradient), painted(&mut d, 20.0, pattern)];
        let mut layer = (*d.layers[0]).clone();
        *layer.children_mut().unwrap() = children;
        d.layers = vec![Arc::new(layer)];
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.len(), 1);
        assert_eq!(out.plan.blocks[0].thread.rgb, [0, 0, 0]);
        assert_eq!(out.warnings, vec!["a stroke with a gradient or pattern was stitched in black".to_string()]);
    }

    /// `doc_with(art)` with its one layer changed by `f`.
    fn layer_with(art: Vec<(PathData, Appearance)>, f: impl FnOnce(&mut NodeKind)) -> Document {
        let mut d = doc_with(art);
        let mut layer = (*d.layers[0]).clone();
        f(&mut layer.kind);
        d.layers = vec![Arc::new(layer)];
        d
    }

    #[test]
    fn template_and_non_printing_layers_are_not_stitched() {
        let art = || vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)];
        let template = layer_with(art(), |k| {
            if let NodeKind::Layer { template, .. } = k {
                *template = true;
            }
        });
        let hidden = layer_with(art(), |k| {
            if let NodeKind::Layer { printable, .. } = k {
                *printable = false;
            }
        });
        for d in [template, hidden] {
            assert_eq!(from_document(&d, &FromDocOptions::default()).unwrap_err(), NOTHING);
        }
        assert!(from_document(&doc_with(art()), &FromDocOptions::default()).is_ok());
    }

    #[test]
    fn clipping_paths_are_not_stitched_and_clipped_art_is_stitched_in_full_with_a_warning() {
        let mut d = Document::new(500.0, 500.0);
        let (p, a) = stroked(square_mm(5.0), BLUE);
        let clip = Node::path(d.alloc_id(), p, a);
        let (p, a) = stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED);
        let art = Node::path(d.alloc_id(), p, a);
        let mut group = Node::group(d.alloc_id(), vec![Arc::new(clip), Arc::new(art)]);
        group.set_clips(true);
        // A path marked as a clipping path, outside a clip group.
        let (p, a) = stroked(line_mm(0.0, 30.0, 10.0, 30.0), BLUE);
        let mut marked = Node::new(d.alloc_id(), NodeKind::Path { path: p, rule: Default::default(), live: None, clipping: true, guide: false });
        marked.appearance = a.into();
        let mut layer = Node::layer(d.alloc_id(), "Art", LayerColor::Preset(0));
        *layer.children_mut().unwrap() = vec![Arc::new(group), Arc::new(marked)];
        d.layers = vec![Arc::new(layer)];
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.len(), 1, "{:?}", out.plan);
        assert_eq!(out.plan.blocks[0].thread.rgb, RED);
        assert_eq!(out.plan.points().last(), Some(Pt { x: 100, y: 0 }), "the clipped line in full");
        assert_eq!(out.warnings, vec!["clipped art was stitched in full: the clip was ignored".to_string()]);
        // A clipping layer clips by its first child in the same way.
        let mut d = doc_with(vec![stroked(square_mm(5.0), BLUE), stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]);
        let mut layer = (*d.layers[0]).clone();
        layer.set_clips(true);
        d.layers = vec![Arc::new(layer)];
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.plan.blocks.iter().map(|b| b.thread.rgb).collect::<Vec<_>>(), vec![RED]);
        assert_eq!(out.warnings.len(), 1);
    }

    #[test]
    fn flattened_points_are_bounded_before_stitching() {
        // A quarter circle flattens to several points but sews only two 12 mm stitches.
        let k = 0.552_284_749_8 * 10.0;
        let sp = SubPath::new(
            vec![Anchor::with_handles(mm(10.0, 0.0), mm(10.0, 0.0), mm(10.0, k)), Anchor::with_handles(mm(0.0, 10.0), mm(k, 10.0), mm(0.0, 10.0))],
            false,
        );
        assert_eq!(run_of(&sp, 120, 3).unwrap_err(), TOO_MANY_STEPS);
        assert_eq!(run_of(&sp, 120, MAX_STEPS).unwrap().len(), 3);
        // The running total counts: a walk that already holds nearly the limit stops at the
        // next path's points.
        let d = doc_with(vec![stroked(PathData::single(SubPath::polyline(&[mm(0.0, 0.0), mm(1.0, 0.0), mm(2.0, 0.0), mm(3.0, 0.0)], false)), RED)]);
        let o = FromDocOptions { stitch_length: 120, ..Default::default() };
        let mut w = Walk::new(&o);
        w.steps = MAX_STEPS - 3;
        assert_eq!(w.node(&d.layers[0], Under::default()).unwrap_err(), TOO_MANY_STEPS);
    }

    #[test]
    fn other_kinds_are_counted_in_a_warning() {
        let mut d = doc_with(vec![stroked(line_mm(0.0, 0.0, 10.0, 0.0), RED)]);
        let symbol = Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "s".into(), xf: Default::default() }));
        let mut layer = (*d.layers[0]).clone();
        layer.children_mut().unwrap().push(symbol);
        d.layers = vec![Arc::new(layer)];
        let out = from_document(&d, &FromDocOptions::default()).unwrap();
        assert_eq!(out.warnings, vec!["1 objects of other kinds (type, images, symbols) were not stitched".to_string()]);
    }

    #[test]
    fn a_curve_is_followed_within_tolerance() {
        // A quarter circle of radius 10 mm about the origin, from (10, 0) to (0, 10).
        let k = 0.552_284_749_8 * 10.0;
        let sp = SubPath::new(
            vec![Anchor::with_handles(mm(10.0, 0.0), mm(10.0, 0.0), mm(10.0, k)), Anchor::with_handles(mm(0.0, 10.0), mm(k, 10.0), mm(0.0, 10.0))],
            false,
        );
        let plan = from_document(&doc_with(vec![stroked(PathData::single(sp), RED)]), &FromDocOptions::default()).unwrap().plan;
        let pts: Vec<Pt> = plan.points().collect();
        assert_eq!(pts.first(), Some(&Pt { x: 100, y: 0 }));
        assert_eq!(pts.last(), Some(&Pt { x: 0, y: 100 }));
        // Arc length 15.7 mm at 2.5 mm: 7 steps.
        assert_eq!(pts.len(), 8);
        for p in &pts {
            let r = f64::from(p.x).hypot(f64::from(p.y));
            assert!((r - 100.0).abs() < 1.5, "{p:?} is {r} units from the centre");
        }
    }

    /// A filled ring outlined in satin, at any angle and size: no stitch is past the machine's
    /// maximum, so `prepare` has none to split, and it warns of nothing.
    mod prepared {
        use proptest::prelude::*;

        use super::*;
        use crate::{COMMERCIAL15, FillParams, PrepareOptions, prepare};

        fn ring(w: f64, h: f64) -> PathData {
            let outer = SubPath::polyline(&[mm(0.0, 0.0), mm(w, 0.0), mm(w, h), mm(0.0, h)], true);
            let inner = SubPath::polyline(&[mm(w * 0.3, h * 0.3), mm(w * 0.7, h * 0.3), mm(w * 0.7, h * 0.7), mm(w * 0.3, h * 0.7)], true);
            PathData::new(vec![outer, inner])
        }

        /// The longest stitch from one needle point to the next, in plan units.
        fn longest(plan: &crate::StitchPlan) -> f64 {
            let mut at: Option<Pt> = None;
            let mut most = 0.0f64;
            for s in plan.blocks.iter().flat_map(|b| b.steps.iter()) {
                match s {
                    Step::Stitch(p) => {
                        if let Some(a) = at {
                            most = most.max(a.dist(*p));
                        }
                        at = Some(*p);
                    }
                    Step::Jump(p) => at = Some(*p),
                    _ => {}
                }
            }
            most
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(24))]

            #[test]
            fn fills_and_satin_pass_prepare_unchanged(w in 10.0f64..80.0, h in 10.0f64..80.0, angle in -90.0f64..90.0, width in 1.0f64..15.0) {
                let mut look = Appearance::basic(Paint::solid(Color::rgb8(0, 0, 255)), Paint::solid(Color::rgb8(255, 0, 0)), 1.0);
                look.stroke_mut().unwrap().width = width * PT_PER_MM;
                let mut d = doc_with(vec![(ring(w, h), look)]);
                let mut layer = (*d.layers[0]).clone();
                let mut node = (*layer.children().unwrap()[0]).clone();
                if let NodeKind::Path { rule, .. } = &mut node.kind {
                    *rule = FillRule::EvenOdd;
                }
                *layer.children_mut().unwrap() = vec![Arc::new(node)];
                d.layers = vec![Arc::new(layer)];
                let o = FromDocOptions {
                    fill: FillOptions { kind: FillType::Tatami, params: FillParams { angle, ..Default::default() } },
                    stroke: StrokeOptions { kind: StrokeType::Satin, ..Default::default() },
                    ..Default::default()
                };
                let made = from_document(&d, &o).unwrap();
                prop_assert!(made.warnings.is_empty(), "{:?}", made.warnings);
                let prepared = prepare(&made.plan, &COMMERCIAL15, &PrepareOptions { locks: true, travel_trims: true }).unwrap();
                prop_assert!(prepared.warnings.is_empty(), "{:?}", prepared.warnings);
                prop_assert!(longest(&made.plan) <= f64::from(COMMERCIAL15.max_stitch), "{}", longest(&made.plan));
            }
        }
    }
}
