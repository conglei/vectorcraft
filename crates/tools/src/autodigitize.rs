//! Auto-Digitize tool (StitchCraft, E10): a click on a shape replaces it with the stitch objects
//! its width calls for (`stitch.autoDigitize` on the object clicked); Enter applies it to the
//! selection. Embroidery › Auto-Digitize Tool chooses it; it is not in the toolbar.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use crate::{Action, Cursor, Mods, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "autoDigitize";
/// The command it runs.
const COMMAND: &str = "stitch.autoDigitize";

/// Create the Auto-Digitize tool by id (None = not ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    (id == ID).then(|| Box::new(AutoDigitizeTool) as Box<dyn Tool>)
}

/// The object a click at `p` digitizes: the topmost object under it.
fn target(cx: &ToolContext, p: Point) -> Option<NodeId> {
    let h = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options())?;
    Some(h.top_object(cx.isolation))
}

#[derive(Default)]
pub struct AutoDigitizeTool;

impl Tool for AutoDigitizeTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Up {
            return vec![];
        }
        match target(cx, ev.pos) {
            Some(id) => vec![Action::Exec(COMMAND.into(), json!({"ids": [id.0]}))],
            None => vec![],
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Enter && !cx.selection.objects.is_empty() {
            return vec![Action::Exec(COMMAND.into(), json!({}))];
        }
        vec![]
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Enter && !cx.selection.objects.is_empty()
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if target(cx, p).is_some() { Cursor::AutoDigitize } else { Cursor::NotAllowed }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_doc::Selection;

    use super::*;
    use crate::testutil::{cx, doc_with_rect, paint};

    #[test]
    fn a_click_digitizes_the_object_under_the_pointer() {
        let (doc, id) = doc_with_rect();
        let sel = Selection::default();
        let p = paint();
        let cx = cx(&doc, &sel, &p);
        let mut t = AutoDigitizeTool;
        let at = Point::new(150.0, 150.0);
        assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::AutoDigitize);
        assert_eq!(t.cursor(&cx, Point::new(400.0, 400.0), Mods::default()), Cursor::NotAllowed);
        let ev = |kind| PointerEvent { kind, pos: at, mods: Mods::default(), backquote: false, pressure: 1.0, pen: None };
        assert!(t.pointer(&cx, &ev(PointerKind::Down)).is_empty());
        match t.pointer(&cx, &ev(PointerKind::Up)).as_slice() {
            [Action::Exec(c, p)] => assert_eq!((c.as_str(), p.clone()), (COMMAND, json!({"ids": [id.0]}))),
            other => panic!("{other:?}"),
        }
        // Enter with nothing selected does nothing.
        assert!(t.key(&cx, ToolKey::Enter, Mods::default()).is_empty());
        assert!(create(ID).is_some());
    }
}
