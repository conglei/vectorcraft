//! The sewing order of a document's objects ([`vectorcraft_doc::StitchDesign`]) and the way each
//! is sewn (its entry and exit points, [`vectorcraft_doc::StitchProps::entry`]): the walk collects
//! each object's runs as a [`Unit`]; they are then sewn in the design's order, each turned to
//! enter and leave where its points say.

use vectorcraft_doc::{Document, Node, NodeId};

use super::{Join, Mode, Walk, collect_subpaths, flatten};
use crate::sequence::{Ends, Item, MAX_WAYS, Way, current_ways, nearest_way, resolve_order};
use crate::{MAX_STEPS, Pt};

/// One run an object sews.
#[derive(Clone, Debug)]
pub(super) struct Run {
    pub(super) rgb: [u8; 3],
    pub(super) pts: Vec<Pt>,
    pub(super) mode: Mode,
    pub(super) join: Join,
    /// Which stitch type of the object made it (an automatic object's fill, then its outline).
    pub(super) part: usize,
}

/// A sewn object: its runs in the order they are made, and where it should be entered and left.
#[derive(Clone, Debug)]
pub(super) struct Unit {
    pub(super) id: NodeId,
    pub(super) runs: Vec<Run>,
    /// The entry and exit the user set (the object is pinned to them).
    pub(super) pin: (Option<Pt>, Option<Pt>),
    /// The entry and exit closest join chose (preferred, chosen again by the next closest join).
    pub(super) prefer: (Option<Pt>, Option<Pt>),
    /// Sewn forwards only (lettering, whose letters are in their own order).
    pub(super) fixed: bool,
}

/// How a unit's runs are turned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Orient {
    Forward,
    /// Each stitch type's runs in reverse order, each run backwards; the types keep their order
    /// (a fill stays under its outline).
    Reversed,
    /// A closed outline started from its stitch point `k`.
    Rotated(usize),
}

/// A closed run: it ends where it starts, with at least two other points.
fn closed(r: &Run) -> bool {
    r.pts.len() >= 4 && r.pts.first() == r.pts.last()
}

impl Unit {
    /// From an opened stitch file: its needle points are sewn as the file had them.
    fn opened(&self) -> bool {
        self.runs.iter().any(|r| matches!(r.join, Join::Opened { .. }))
    }

    /// The ways the unit may be turned: forwards only for an opened file's art, else forwards,
    /// backwards and, for one closed run, from up to [`MAX_WAYS`] − 2 stitch points around it
    /// plus the one nearest each of its points.
    fn orients(&self) -> Vec<Orient> {
        if self.opened() || self.fixed {
            return vec![Orient::Forward];
        }
        let mut v = vec![Orient::Forward, Orient::Reversed];
        if let [only] = self.runs.as_slice()
            && closed(only)
        {
            // The distinct points: the last repeats the first.
            let n = only.pts.len() - 1;
            let step = n.div_ceil(MAX_WAYS - 1).max(1);
            v.extend((step..n).step_by(step).take(MAX_WAYS - 2).map(Orient::Rotated));
            for p in [self.pin.0, self.pin.1, self.prefer.0, self.prefer.1].into_iter().flatten() {
                let nearest = only.pts.iter().take(n).enumerate().min_by(|a, b| a.1.dist(p).total_cmp(&b.1.dist(p))).map(|(i, _)| i);
                if let Some(k) = nearest.filter(|&k| k > 0)
                    && !v.contains(&Orient::Rotated(k))
                {
                    v.push(Orient::Rotated(k));
                }
            }
        }
        v
    }

    /// The runs turned by `o`.
    fn oriented(&self, o: Orient) -> Vec<Run> {
        match o {
            Orient::Forward => self.runs.clone(),
            Orient::Reversed => {
                let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
                for group in self.runs.chunk_by(|a, b| a.part == b.part) {
                    out.extend(group.iter().rev().map(|r| {
                        let mut r = r.clone();
                        r.pts.reverse();
                        r
                    }));
                }
                out
            }
            Orient::Rotated(k) => {
                let mut out = self.runs.clone();
                for r in &mut out {
                    let n = r.pts.len().saturating_sub(1);
                    if closed(r) && k < n {
                        let mut pts: Vec<Pt> = r.pts.get(k..n).unwrap_or(&[]).to_vec();
                        pts.extend_from_slice(r.pts.get(..=k).unwrap_or(&[]));
                        r.pts = pts;
                    }
                }
                out
            }
        }
    }

    /// Where the unit is entered and left turned by `o`, and the trims and travel between its own
    /// runs, runs that start within `min_stitch` of the last stitch being joined.
    fn way(&self, o: Orient, min_stitch: i32) -> Option<Way> {
        let runs = self.oriented(o);
        let mut sewn = runs.iter().filter(|r| !r.pts.is_empty());
        let first = sewn.next()?;
        let entry = *first.pts.first()?;
        let mut exit = *first.pts.last()?;
        let mut rgb = first.rgb;
        let (mut trims, mut travel) = (0usize, 0.0f64);
        for r in sewn {
            let (Some(&a), Some(&b)) = (r.pts.first(), r.pts.last()) else { continue };
            let d = exit.dist(a);
            travel += d;
            if r.rgb == rgb && r.join.trims(d, min_stitch) {
                trims += 1;
            }
            exit = b;
            rgb = r.rgb;
        }
        Some(Way { entry, exit, trims, travel })
    }

    /// The threads the unit sews in, in order, each once per change.
    fn colours(&self) -> Vec<[u8; 3]> {
        let mut v: Vec<[u8; 3]> = vec![];
        for r in self.runs.iter().filter(|r| !r.pts.is_empty()) {
            if v.last() != Some(&r.rgb) {
                v.push(r.rgb);
            }
        }
        v
    }
}

/// An object in the sewing order ([`sequence`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SequenceItem {
    pub id: NodeId,
    /// How it can be sewn; [`SequenceItem::way`] is how it is.
    pub item: Item,
    pub way: usize,
    /// The threads it sews in, in order.
    pub colours: Vec<[u8; 3]>,
    /// Needle points it sews, lock stitches and the machine's preparation aside.
    pub stitches: usize,
    /// Its art came from an opened stitch file: sewn as the file had it.
    pub opened: bool,
    /// The user set its entry or exit point.
    pub pinned: bool,
}

impl SequenceItem {
    /// Where it is entered and left, the way it is sewn.
    pub fn ends(&self) -> Option<(Pt, Pt)> {
        let w = self.item.ways.get(self.way)?;
        Some((w.entry, w.exit))
    }
}

/// A document's objects in sewing order ([`sequence`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Sequenced {
    pub items: Vec<SequenceItem>,
    /// The sewn objects in document order, bottom to top.
    pub natural: Vec<NodeId>,
    /// The design's start and end points and the join distance.
    pub ends: Ends,
}

/// A plan point of document point `p`.
fn plan_pt(p: vectorcraft_geom::Point) -> Option<Pt> {
    Pt::from_pt(p.x, p.y)
}

/// The design's start and end, and the join distance `min_stitch`.
pub(super) fn ends_of(doc: &Document, min_stitch: i32) -> Ends {
    Ends { start: doc.stitch.start_point().and_then(plan_pt), end: doc.stitch.end_point().and_then(plan_pt), min_stitch }
}

/// The units in sewing order: (unit index, orient) pairs, and the items, ways and choices.
struct Ordered {
    order: Vec<usize>,
    items: Vec<Item>,
    orients: Vec<Vec<Orient>>,
    ways: Vec<usize>,
}

/// The sewing order of `units`: the design's order, each unit turned to its pinned or preferred
/// entry and exit, the first and last of the others to the design's start and end.
fn order(doc: &Document, units: &[Unit], ends: &Ends) -> Ordered {
    let natural: Vec<NodeId> = units.iter().map(|u| u.id).collect();
    let order = resolve_order(&doc.stitch.order, &natural);
    let mut items = Vec::with_capacity(order.len());
    let mut orients = Vec::with_capacity(order.len());
    let mut chosen = Vec::with_capacity(order.len());
    for u in order.iter().filter_map(|&i| units.get(i)) {
        let mut os = u.orients();
        let mut ways: Vec<Way> = vec![];
        let mut kept: Vec<Orient> = vec![];
        for o in os.drain(..) {
            if let Some(w) = u.way(o, ends.min_stitch) {
                ways.push(w);
                kept.push(o);
            }
        }
        let mut pick = None;
        if let Some(w) = nearest_way(&ways, u.pin.0, u.pin.1) {
            // Pinned: the one way nearest the user's points.
            ways = ways.get(w..=w).map(<[Way]>::to_vec).unwrap_or_default();
            kept = kept.get(w..=w).map(<[Orient]>::to_vec).unwrap_or_default();
            pick = Some(0);
        } else if let Some(w) = nearest_way(&ways, u.prefer.0, u.prefer.1) {
            pick = Some(w);
        }
        let colours = u.colours();
        let first_rgb = colours.first().copied().unwrap_or_default();
        let last_rgb = colours.last().copied().unwrap_or_default();
        items.push(Item { first_rgb, last_rgb, changes: colours.len().saturating_sub(1), ways });
        orients.push(kept);
        chosen.push(pick);
    }
    let ways = current_ways(&items, &chosen, ends);
    Ordered { order, items, orients, ways }
}

impl Walk<'_> {
    /// Sew the collected units in the design's order, each turned as [`order`] says.
    pub(super) fn sew_units(&mut self, doc: &Document) -> Result<(), String> {
        let units = std::mem::take(&mut self.units);
        let ends = ends_of(doc, self.o.min_stitch);
        let o = order(doc, &units, &ends);
        self.steps = 0;
        for (k, &i) in o.order.iter().enumerate() {
            let Some(u) = units.get(i) else { continue };
            let way = o.ways.get(k).copied().unwrap_or(0);
            let orient = o.orients.get(k).and_then(|os| os.get(way)).copied().unwrap_or(Orient::Forward);
            for r in u.oriented(orient) {
                self.emit_run(r.rgb, &r.pts, r.mode, r.join)?;
            }
        }
        Ok(())
    }
}

/// `doc`'s sewn objects in sewing order, as [`crate::from_document`] with options `o` sews them:
/// each with the ways it can be sewn and the way it is.
pub fn sequence(doc: &Document, o: &super::FromDocOptions) -> Result<Sequenced, String> {
    let mut w = Walk::new(o);
    for layer in &doc.layers {
        w.node(layer, super::Under::default())?;
    }
    let ends = ends_of(doc, o.min_stitch);
    let ord = order(doc, &w.units, &ends);
    let natural = w.units.iter().map(|u| u.id).collect();
    let mut items = Vec::with_capacity(ord.order.len());
    for ((&i, item), way) in ord.order.iter().zip(ord.items).zip(ord.ways) {
        let Some(u) = w.units.get(i) else { continue };
        items.push(SequenceItem {
            id: u.id,
            item,
            way,
            colours: u.colours(),
            stitches: u.runs.iter().map(|r| r.pts.len()).sum(),
            opened: u.opened(),
            pinned: u.pin.0.is_some() || u.pin.1.is_some(),
        });
    }
    Ok(Sequenced { items, natural, ends })
}

/// Object `n`'s outline: each subpath flattened to millimetres, a closed one back to its start.
fn outline(n: &Node) -> Option<Vec<Vec<(f64, f64)>>> {
    let mut sps = vec![];
    collect_subpaths(n, &mut sps);
    let mut out = vec![];
    for sp in sps {
        let (mut pts, _) = flatten(sp, MAX_STEPS).ok()?;
        if sp.closed
            && sp.segment_count() > 0
            && let Some(&first) = pts.first()
        {
            pts.push(first);
        }
        out.push(pts);
    }
    Some(out)
}

fn seg_len(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The point a fraction `t` (0–1) of the way along object `n`'s outline (its subpaths one after
/// another, by arc length), as a plan point. `None`: `n` has no outline, or `t` is out of range.
pub fn outline_point(n: &Node, t: f64) -> Option<Pt> {
    if !(t.is_finite() && (0.0..=1.0).contains(&t)) {
        return None;
    }
    let rings = outline(n)?;
    let total: f64 = rings.iter().flat_map(|r| r.windows(2)).filter_map(|w| if let &[a, b] = w { Some(seg_len(a, b)) } else { None }).sum();
    let mut left = t * total;
    let mut last = None;
    for ring in &rings {
        for w in ring.windows(2) {
            let &[a, b] = w else { continue };
            let l = seg_len(a, b);
            if left <= l && l > 0.0 {
                let f = left / l;
                return Pt::from_mm(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            }
            left -= l;
            last = Some(b);
        }
        if last.is_none() {
            last = ring.first().copied();
        }
    }
    last.and_then(|(x, y)| Pt::from_mm(x, y))
}

/// The fraction (0–1) of the way along object `n`'s outline of the outline point nearest `p`
/// ([`outline_point`] reversed). `None`: `n` has no outline.
pub fn outline_fraction(n: &Node, p: Pt) -> Option<f64> {
    let rings = outline(n)?;
    let (px, py) = p.to_mm();
    let mut walked = 0.0;
    let mut best: Option<(f64, f64)> = None;
    for ring in &rings {
        if let [only] = ring.as_slice() {
            let d = seg_len(*only, (px, py));
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, walked));
            }
        }
        for w in ring.windows(2) {
            let &[a, b] = w else { continue };
            let l = seg_len(a, b);
            let f = if l > 0.0 { (((px - a.0) * (b.0 - a.0) + (py - a.1) * (b.1 - a.1)) / (l * l)).clamp(0.0, 1.0) } else { 0.0 };
            let q = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
            let d = seg_len(q, (px, py));
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, walked + l * f));
            }
            walked += l;
        }
    }
    let (_, at) = best?;
    Some(if walked > 0.0 { (at / walked).clamp(0.0, 1.0) } else { 0.0 })
}

/// The plan points of object `n`'s fractions `t` along its outline.
pub(super) fn points_of(n: &Node, t: (Option<f64>, Option<f64>)) -> (Option<Pt>, Option<Pt>) {
    (t.0.and_then(|t| outline_point(n, t)), t.1.and_then(|t| outline_point(n, t)))
}
