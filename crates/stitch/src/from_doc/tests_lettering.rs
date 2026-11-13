//! Type sewn as lettering by [`crate::from_document`], with the bundled Source Sans 3.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Document, LayerColor, LetterTrims, Node, NodeId, NodeKind, StitchProps, StitchType, TextObject};
use vectorcraft_geom::{PathData, Point, SubPath};

use super::*;
use crate::{LetterMode, PT_PER_MM, UNITS_PER_MM, stats};

/// A document with one point-type object of `text` at font size `size_mm`, its stitch
/// properties `props`.
fn text_doc(text: &str, size_mm: f64, props: StitchProps) -> Document {
    let style = CharStyle { size: size_mm * PT_PER_MM, fill: Paint::solid(Color::rgb8(200, 0, 0)), ..CharStyle::default() };
    let mut n = Node::new(NodeId(1), NodeKind::Text(Box::new(TextObject::point(Point::new(30.0, 100.0), text, style))));
    n.set_stitch_props(props);
    let mut doc = Document::new(800.0, 400.0);
    let mut layer = Node::layer(NodeId(100), "Art", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = vec![Arc::new(n)];
    }
    doc.layers = vec![Arc::new(layer)];
    doc
}

fn plan_of(doc: &Document) -> Planned {
    from_document(doc, &FromDocOptions::default()).unwrap()
}

fn lettered(doc: &Document) -> Lettered {
    let n = doc.node(NodeId(1)).unwrap();
    let props = n.stitch_props().copied().unwrap_or_default();
    lettering_of(n, &props, &FromDocOptions::default(), MAX_STEPS).unwrap().unwrap()
}

#[test]
fn hello_at_15_mm_is_satin_lettering_with_one_trim_per_word() {
    let doc = text_doc("Hello", 15.0, StitchProps::default());
    let l = lettered(&doc);
    assert_eq!(l.mode, LetterMode::Satin, "cap height {}", l.height_mm);
    assert!((6.0..12.0).contains(&l.height_mm), "{}", l.height_mm);
    assert_eq!(l.letters.len(), 5);
    for s in &l.letters {
        assert_eq!(s.mode, LetterMode::Satin);
        assert!(s.columns >= 1, "{s:?}");
    }
    let out = plan_of(&doc);
    assert!(out.warnings.iter().all(|w| !w.contains("not stitched")), "{:?}", out.warnings);
    let st = stats(&out.plan);
    assert_eq!(st.trims, 0, "one word: no trim between its letters");
    assert_eq!(out.plan.blocks.len(), 1);
    assert_eq!(out.plan.blocks[0].thread.rgb, [200, 0, 0]);
    assert!(st.stitches > 400, "{}", st.stitches);
    // Two words: one trim between them; per letter: one between every two letters.
    let two = plan_of(&text_doc("Hello you", 15.0, StitchProps::default()));
    assert_eq!(stats(&two.plan).trims, 1);
    let per_letter = plan_of(&text_doc("Hello you", 15.0, StitchProps { letter_trims: Some(LetterTrims::Letter), ..Default::default() }));
    assert_eq!(stats(&per_letter.plan).trims, 7);
    let never = plan_of(&text_doc("Hello you", 15.0, StitchProps { letter_trims: Some(LetterTrims::None), ..Default::default() }));
    assert_eq!(stats(&never.plan).trims, 0);
}

#[test]
fn small_type_is_running_stitch_and_middling_type_satin() {
    let small = lettered(&text_doc("Test", 5.0, StitchProps::default()));
    assert_eq!(small.mode, LetterMode::Run, "height {}", small.height_mm);
    assert!(small.letters.iter().all(|l| l.columns == 0));
    let mid = lettered(&text_doc("Test", 12.0, StitchProps::default()));
    assert_eq!(mid.mode, LetterMode::Satin, "height {}", mid.height_mm);
    // The run height is a parameter.
    let low = lettered(&text_doc("Test", 5.0, StitchProps { run_under: Some(2.0), ..Default::default() }));
    assert_eq!(low.mode, LetterMode::Satin);
}

#[test]
fn stitch_types_on_type() {
    let none = text_doc("Hi", 15.0, StitchProps { kind: StitchType::None, ..Default::default() });
    assert!(from_document(&none, &FromDocOptions::default()).is_err(), "nothing to sew");
    let fill = lettered(&text_doc("Hi", 15.0, StitchProps { kind: StitchType::Tatami, ..Default::default() }));
    assert!(fill.letters.iter().all(|l| l.mode == LetterMode::Fill));
    let run = lettered(&text_doc("Hi", 15.0, StitchProps { kind: StitchType::Run, ..Default::default() }));
    assert_eq!(run.mode, LetterMode::Run);
    let manual = plan_of(&text_doc("Hi", 15.0, StitchProps { kind: StitchType::Manual, ..Default::default() }).clone_with_square());
    assert!(manual.warnings.iter().any(|w| w.contains("not stitched")));
}

trait WithSquare {
    fn clone_with_square(&self) -> Document;
}

impl WithSquare for Document {
    /// The document with a stroked square added, so it has something to sew.
    fn clone_with_square(&self) -> Document {
        let mut d = self.clone();
        let sq = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(30.0, 30.0)], true));
        let n = Node::path(NodeId(50), sq, vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::rgb8(0, 0, 0)), 1.0));
        let mut layer = (*d.layers[0]).clone();
        if let Some(c) = layer.children_mut() {
            c.push(Arc::new(n));
        }
        d.layers = vec![Arc::new(layer)];
        d
    }
}

#[test]
fn two_rails_with_satin_sew_one_column_between_them() {
    let mm = |x: f64, y: f64| Point::new(x * PT_PER_MM, y * PT_PER_MM);
    let mut path = PathData::single(SubPath::polyline(&[mm(0.0, 0.0), mm(20.0, 0.0)], false));
    // The second rail drawn the other way round.
    path.subpaths.push(SubPath::polyline(&[mm(20.0, 3.0), mm(0.0, 3.0)], false));
    let mut n = Node::path(NodeId(1), path, vectorcraft_doc::Appearance::basic(Paint::None, Paint::solid(Color::rgb8(0, 0, 0)), 1.0));
    let props = StitchProps { kind: StitchType::Satin, compensation: Some(0.2), ..Default::default() };
    n.set_stitch_props(props);
    let mut doc = Document::new(800.0, 400.0);
    let mut layer = Node::layer(NodeId(100), "Art", LayerColor::Preset(0));
    if let Some(c) = layer.children_mut() {
        *c = vec![Arc::new(n.clone())];
    }
    doc.layers = vec![Arc::new(layer.clone())];
    let p = plan_of(&doc).plan;
    let st = stats(&p);
    assert_eq!(st.trims, 0);
    // Zig-zag across the 3 mm gap (plus 0.2 mm each side), not two columns round the rails.
    let ys: Vec<i32> = p.blocks[0].steps.iter().filter_map(|s| if let Step::Stitch(q) = s { Some(q.y) } else { None }).collect();
    assert_eq!((ys.iter().min(), ys.iter().max()), (Some(&-2), Some(&32)));
    assert!(st.stitches > 90 && st.stitches < 110, "{}", st.stitches);
    // With underlay, a centre run first.
    let mut under = n;
    under.set_stitch_props(StitchProps { underlay: Some(vectorcraft_doc::Underlay::Center), ..props });
    if let Some(c) = layer.children_mut() {
        *c = vec![Arc::new(under)];
    }
    doc.layers = vec![Arc::new(layer)];
    let p = plan_of(&doc).plan;
    let first = p.blocks[0].steps.iter().find_map(|s| if let Step::Stitch(q) = s { Some(*q) } else { None }).unwrap();
    assert_eq!(first.y, 15, "the underlay runs down the middle");
    // Never turned round (the underlay would end up over the satin).
    let seq = sequence(&doc, &FromDocOptions::default()).unwrap();
    assert_eq!(seq.items[0].item.ways.len(), 1);
}

/// Jumps the machine would sew as a loose thread: the first jump of a travel that no trim
/// precedes, longer than the gap a stitch may cross.
fn bare_jumps(plan: &crate::StitchPlan) -> Vec<f64> {
    let mut out = vec![];
    for b in &plan.blocks {
        let mut pos: Option<Pt> = None;
        let mut cut = true;
        let mut in_jump = false;
        for s in &b.steps {
            match s {
                Step::Trim => cut = true,
                Step::Jump(q) => {
                    if let (false, false, Some(p)) = (in_jump, cut, pos) {
                        let d = p.dist(*q) / UNITS_PER_MM;
                        if d > crate::lettering::NO_TRIM_GAP_MM {
                            out.push(d);
                        }
                    }
                    in_jump = true;
                    pos = Some(*q);
                }
                Step::Stitch(q) => {
                    cut = false;
                    in_jump = false;
                    pos = Some(*q);
                }
                Step::Stop => {}
            }
        }
    }
    out
}

/// Found at the first machine preview (2026-11-13): letters of a word and the parts of a letter
/// were reached by a single jump, which the machine sews as a thread floating across the fabric.
/// Now a word's letters are joined by a walk of stitches, and a part by a walk inside the letter
/// or by a trim.
#[test]
fn lettering_never_leaves_a_bare_jump() {
    for (text, size, trims) in [
        ("Hello", 15.0, None),
        ("Hello you", 15.0, None),
        ("Hello you", 15.0, Some(LetterTrims::None)),
        ("Hello you", 15.0, Some(LetterTrims::Letter)),
        ("ij", 15.0, None),
        ("Test", 5.0, None),
        ("Test", 12.0, None),
        ("I\u{2019}m Conglei", 12.0, None),
    ] {
        let doc = text_doc(text, size, StitchProps { letter_trims: trims, ..Default::default() });
        let out = plan_of(&doc);
        assert_eq!(bare_jumps(&out.plan), Vec::<f64>::new(), "{text:?} at {size} mm, trims {trims:?}");
    }
    // Letters of a word are walked to, stitching: the plan holds stitches between the letters.
    let l = lettered(&text_doc("Hello", 15.0, StitchProps::default()));
    assert!(l.letters.iter().skip(1).all(|s| s.joined), "{:?}", l.letters.iter().map(|s| s.joined).collect::<Vec<_>>());
    assert!(!l.letters[0].joined);
    // An i's dot is a part no walk inside the ink reaches: a trim, not a float.
    let dots = plan_of(&text_doc("ij", 15.0, StitchProps::default()));
    assert!(stats(&dots.plan).trims >= 2, "{}", stats(&dots.plan).trims);
    // Per letter: nothing is joined; per word: a new word is not.
    let per_letter = lettered(&text_doc("Hello you", 15.0, StitchProps { letter_trims: Some(LetterTrims::Letter), ..Default::default() }));
    assert!(per_letter.letters.iter().all(|s| !s.joined));
    let words = lettered(&text_doc("Hello you", 15.0, StitchProps::default()));
    let joined: Vec<bool> = words.letters.iter().map(|s| s.joined).collect();
    assert_eq!(joined, vec![false, true, true, true, true, false, true, true]);
}

/// What Convert to Stitch Objects makes of `doc`'s type, as letters of pieces.
fn pieces_of(doc: &Document) -> Vec<crate::lettering::PieceLetter> {
    use crate::lettering::{PieceLetter, PieceStyle, shape_letter};
    let l = lettered(doc);
    let NodeKind::Text(t) = &doc.node(NodeId(1)).unwrap().kind else { panic!("not type") };
    let style = PieceStyle { spacing: l.params.spacing, compensation: l.params.compensation, underlay: l.params.underlay };
    letters_of(t)
        .unwrap()
        .iter()
        .map(|x| {
            let s = shape_letter(&x.rings, l.mode, &l.params).unwrap();
            PieceLetter {
                rgb: x.rgb,
                word: x.word,
                columns: s.columns.into_iter().map(|c| (c, style)).collect(),
                runs: s.runs,
                fills: if s.fill.is_empty() { vec![] } else { vec![s.fill] },
            }
        })
        .collect()
}

/// STI-38: a letter's pieces, as Convert to Stitch Objects makes them, sew as the letter does:
/// the same parts and about the same stitches ("Hello" within 5 %; a stroke cut into a column and
/// a run at a junction travels a little more as two pieces, up to 10 % on "TEXT").
#[test]
fn pieces_sew_as_the_letters_do() {
    for (text, within) in [("Hello", 0.05), ("Hi", 0.1), ("TEXT", 0.1)] {
        let doc = text_doc(text, 15.0, StitchProps::default());
        let l = lettered(&doc);
        let sewn = crate::lettering::sew_pieces(&pieces_of(&doc), &l.params, MAX_STEPS).unwrap();
        let count = |v: &[crate::SewnLetter]| v.iter().flat_map(|s| &s.runs).map(Vec::len).sum::<usize>();
        let parts = |v: &[crate::SewnLetter]| v.iter().map(|s| s.runs.len()).collect::<Vec<_>>();
        assert_eq!(parts(&sewn), parts(&l.letters), "{text}: runs per letter");
        let (a, b) = (count(&l.letters) as f64, count(&sewn) as f64);
        assert!((b / a - 1.0).abs() <= within, "{text}: {a} live, {b} from pieces");
    }
}
