//! Entry and Exit tool (Embroidery): the selected paths and compound paths show where sewing
//! enters them (a green ring) and leaves them (a red ring), as the sewing order has them. A drag
//! moves a point along the object's outline (`stitch.entryExit.set`, previewed and committed as one
//! undo step); near the previous object's exit (an entry) or the next object's entry (an exit) it
//! snaps there, for a join without travel. Shift moves the entry and exit together.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;
use vectorcraft_stitch::{FromDocOptions, PT_PER_MM, Pt, sequence};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "stitchEntryExit";
/// How near a point the pointer picks it, and snaps to a neighbour's, in screen pixels.
pub const PICK_PX: f64 = 8.0;
const ENTRY: [u8; 3] = [0x2e, 0xb8, 0x6a];
const EXIT: [u8; 3] = [0xd8, 0x3a, 0x3a];
const NEIGHBOUR: [u8; 3] = [0x80, 0x80, 0x80];
const RING_PX: f32 = 9.0;

/// Which point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Entry,
    Exit,
}

impl End {
    fn key(self) -> &'static str {
        match self {
            End::Entry => "entry",
            End::Exit => "exit",
        }
    }
}

/// An object in sewing order, with where it is entered and left (document points).
#[derive(Clone, Debug, PartialEq)]
struct Sewn {
    id: NodeId,
    entry: Point,
    exit: Point,
}

#[derive(Clone, Debug, Default)]
struct Cache {
    revision: (u64, u64),
    sewn: Vec<Sewn>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    /// Index into the cached order.
    at: usize,
    end: End,
    moved: bool,
}

#[derive(Default)]
pub struct EntryExitTool {
    cache: Option<Cache>,
    hover: Option<(usize, End)>,
    drag: Option<Drag>,
    /// Where the dragged point is shown while the drag lasts.
    shown: Option<Point>,
}

fn doc_point(p: Pt) -> Point {
    let (x, y) = p.to_mm();
    Point::new(x * PT_PER_MM, y * PT_PER_MM)
}

/// The sewing order of `cx.doc`, with each object's entry and exit.
fn sewn_of(cx: &ToolContext) -> Vec<Sewn> {
    let Ok(seq) = sequence(cx.doc, &FromDocOptions::default()) else { return vec![] };
    seq.items.iter().filter_map(|it| it.ends().map(|(a, b)| Sewn { id: it.id, entry: doc_point(a), exit: doc_point(b) })).collect()
}

impl EntryExitTool {
    /// The sewing order, made again when the document or the selection changed.
    fn sewn(&mut self, cx: &ToolContext) -> &[Sewn] {
        if self.cache.as_ref().is_none_or(|c| c.revision != cx.revision) {
            self.cache = Some(Cache { revision: cx.revision, sewn: sewn_of(cx) });
        }
        self.cache.as_ref().map(|c| c.sewn.as_slice()).unwrap_or_default()
    }

    /// Whether object `id` shows its points: it, or a path of the compound path it is, is selected.
    fn shows(cx: &ToolContext, id: NodeId) -> bool {
        cx.selection.objects.iter().any(|&s| s == id || cx.doc.parent_of(s) == Some(id))
    }

    /// The selected object's point under `p`, nearest first.
    fn hit(&mut self, cx: &ToolContext, p: Point) -> Option<(usize, End)> {
        let tol = cx.tol(PICK_PX);
        let mut best: Option<(f64, usize, End)> = None;
        for (i, s) in self.sewn(cx).iter().enumerate() {
            if !Self::shows(cx, s.id) {
                continue;
            }
            for (end, q) in [(End::Exit, s.exit), (End::Entry, s.entry)] {
                let d = q.distance(p);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i, end));
                }
            }
        }
        best.map(|(_, i, e)| (i, e))
    }

    /// Where a point dragged to `p` goes: the neighbour's point (the previous object's exit for an
    /// entry, the next object's entry for an exit) when within [`PICK_PX`], else `p`.
    fn snapped(&mut self, cx: &ToolContext, at: usize, end: End, p: Point) -> Point {
        let tol = cx.tol(PICK_PX);
        let sewn = self.sewn(cx);
        let neighbour = match end {
            End::Entry => at.checked_sub(1).and_then(|i| sewn.get(i)).map(|s| s.exit),
            End::Exit => sewn.get(at + 1).map(|s| s.entry),
        };
        neighbour.filter(|q| q.distance(p) <= tol).unwrap_or(p)
    }

    fn params(id: NodeId, end: End, p: Point, both: bool) -> Value {
        let at = json!({"x": p.x, "y": p.y});
        let mut v = json!({ "ids": [id.0] });
        if both {
            v["entry"] = at.clone();
            v["exit"] = at;
        } else {
            v[end.key()] = at;
        }
        v
    }
}

impl Tool for EntryExitTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.drag.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.hover = self.hit(cx, ev.pos);
                vec![]
            }
            PointerKind::Down => {
                let Some((at, end)) = self.hit(cx, ev.pos) else { return vec![] };
                self.drag = Some(Drag { at, end, moved: false });
                self.shown = None;
                let label = match end {
                    End::Entry => "Move Entry Point",
                    End::Exit => "Move Exit Point",
                };
                vec![Action::Begin(label.into())]
            }
            PointerKind::Drag => {
                let Some(drag) = self.drag else { return vec![] };
                let Some(id) = self.sewn(cx).get(drag.at).map(|s| s.id) else { return vec![] };
                let p = self.snapped(cx, drag.at, drag.end, ev.pos);
                self.drag = Some(Drag { moved: true, ..drag });
                self.shown = Some(p);
                vec![Action::Preview("stitch.entryExit.set".into(), Self::params(id, drag.end, p, ev.mods.shift))]
            }
            PointerKind::Up => {
                let Some(drag) = self.drag.take() else { return vec![] };
                self.shown = None;
                vec![if drag.moved { Action::Commit } else { Action::Cancel }]
            }
            PointerKind::DoubleClick => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.drag.take().is_some() {
            self.shown = None;
            return vec![Action::Cancel];
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.drag.is_some() && key == ToolKey::Escape
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if cx.selection.objects.is_empty() {
            return vec![];
        }
        // The order kept for the document as it is, else made now (until the next pointer event
        // keeps it).
        let sewn = match self.cache.as_ref().filter(|c| c.revision == cx.revision) {
            Some(c) => c.sewn.clone(),
            None => sewn_of(cx),
        };
        let ring = |p, color, filled| Overlay::CurvePoint { p, color, filled, size: RING_PX };
        let mut out = vec![];
        for (i, s) in sewn.iter().enumerate() {
            if !Self::shows(cx, s.id) {
                continue;
            }
            // The neighbours' points a drag snaps to, in grey.
            if let Some(prev) = i.checked_sub(1).and_then(|k| sewn.get(k)) {
                out.push(ring(prev.exit, NEIGHBOUR, false));
            }
            if let Some(next) = sewn.get(i + 1) {
                out.push(ring(next.entry, NEIGHBOUR, false));
            }
            for (end, p, color) in [(End::Entry, s.entry, ENTRY), (End::Exit, s.exit, EXIT)] {
                let lit = self.hover == Some((i, end)) || self.drag.is_some_and(|d| d.at == i && d.end == end);
                out.push(ring(p, color, lit));
            }
        }
        if let Some(p) = self.shown {
            out.push(ring(p, ENTRY, true));
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        if self.drag.is_some() || self.hover.is_some() { Cursor::StitchEntryExit } else { Cursor::Arrow }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::{PathData, SubPath};

    /// Two red lines drawn left to right, 0–100 at y = 0 and y = 50: the first is left at
    /// (100, 0) and the second entered at (0, 50).
    fn doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (a, b) = (d.alloc_id(), d.alloc_id());
        let look = Appearance::basic(Paint::None, Paint::solid(Color::rgb8(255, 0, 0)), 1.0);
        for (i, (id, y)) in [(a, 0.0), (b, 50.0)].into_iter().enumerate() {
            let path = PathData::single(SubPath::polyline(&[Point::new(0.0, y), Point::new(100.0, y)], false));
            d.insert(Some(l), i, Node::path(id, path, look.clone())).unwrap();
        }
        (d, a, b)
    }

    fn ev(kind: PointerKind, x: f64, y: f64, shift: bool) -> PointerEvent {
        let mut e = PointerEvent::new(kind, x, y);
        e.mods.shift = shift;
        e
    }

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 0.5
    }

    #[test]
    fn a_selected_objects_points_are_shown_and_hovered() {
        let (d, a, _) = doc();
        let mut sel = Selection::default();
        sel.set([a]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = EntryExitTool::default();
        t.pointer(&cx, &ev(PointerKind::Move, 300.0, 300.0, false));
        assert_eq!(t.cursor(&cx, Point::new(300.0, 300.0), Mods::default()), Cursor::Arrow);
        let rings: Vec<Point> = t.overlays(&cx).iter().filter_map(|o| if let Overlay::CurvePoint { p, .. } = o { Some(*p) } else { None }).collect();
        assert!(rings.iter().any(|p| near(*p, Point::new(0.0, 0.0))) && rings.iter().any(|p| near(*p, Point::new(100.0, 0.0))), "{rings:?}");
        t.pointer(&cx, &ev(PointerKind::Move, 99.0, 1.0, false));
        assert_eq!(t.hover, Some((0, End::Exit)));
        assert_eq!(t.cursor(&cx, Point::new(99.0, 1.0), Mods::default()), Cursor::StitchEntryExit);
        // Nothing selected: no points, nothing to pick.
        let none = Selection::default();
        let cx = crate::testutil::cx(&d, &none, &paint);
        let mut t = EntryExitTool::default();
        assert!(t.overlays(&cx).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 0.0, 0.0, false)).is_empty());
    }

    #[test]
    fn a_drag_previews_and_commits_the_point_and_snaps_to_the_neighbour() {
        let (d, a, b) = doc();
        let mut sel = Selection::default();
        sel.set([b]);
        let paint = paint();
        let cx = cx(&d, &sel, &paint);
        let mut t = EntryExitTool::default();
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 0.0, 50.0, false)), vec![Action::Begin("Move Entry Point".into())]);
        assert!(t.busy());
        // To the middle of the line.
        let r = t.pointer(&cx, &ev(PointerKind::Drag, 50.0, 52.0, false));
        assert_eq!(r, vec![Action::Preview("stitch.entryExit.set".into(), json!({"ids": [b.0], "entry": {"x": 50.0, "y": 52.0}}))]);
        // Near the first line's exit: snapped onto it.
        // (The sewing order's points are on the 0.1 mm stitch grid.)
        let r = t.pointer(&cx, &ev(PointerKind::Drag, 97.0, 3.0, false));
        let [Action::Preview(c, p)] = r.as_slice() else { panic!("{r:?}") };
        assert_eq!(c, "stitch.entryExit.set");
        let at = Point::new(p["entry"]["x"].as_f64().unwrap(), p["entry"]["y"].as_f64().unwrap());
        assert!(near(at, Point::new(100.0, 0.0)), "{p}");
        // Shift: entry and exit together.
        let r = t.pointer(&cx, &ev(PointerKind::Drag, 60.0, 50.0, true));
        let p = json!({"x": 60.0, "y": 50.0});
        assert_eq!(r, vec![Action::Preview("stitch.entryExit.set".into(), json!({"ids": [b.0], "entry": p, "exit": p}))]);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 60.0, 50.0, true)), vec![Action::Commit]);
        assert!(!t.busy());
        // A press and release without a move changes nothing; Escape cancels a drag.
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 50.0, false));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 0.0, 50.0, false)), vec![Action::Cancel]);
        t.pointer(&cx, &ev(PointerKind::Down, 0.0, 50.0, false));
        assert!(t.claims_key(&cx, ToolKey::Escape));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        let _ = a;
    }
}
