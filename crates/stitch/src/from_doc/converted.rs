//! Converted lettering (`text.toStitches`): a group whose own stitch type is lettering, holding a
//! group per word, each holding a group per letter, each holding the letter's pieces (two-rail
//! satin columns, running stitch lines, fills). It is sewn as one lettering object: the pieces are
//! gathered back into strokes and sewn by lettering's router ([`crate::lettering::sew_pieces`]),
//! with its walks, underlay and trims per word.

use vectorcraft_doc::{Node, NodeKind, StitchParam, StitchProps, StitchType, Underlay};

use super::{Under, Walk, collect_subpaths, flatten, paired_rails, thread, two_rails};
use crate::MAX_STEPS;
use crate::lettering::{Column, P, PieceLetter, PieceStyle, sew_pieces};

/// Whether `n` is converted lettering: a group whose own stitch type is lettering.
pub(super) fn is_converted(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. }) && n.stitch_props().is_some_and(|p| p.kind == StitchType::Lettering)
}

/// The visible children of group `n`.
fn children(n: &Node) -> Vec<&Node> {
    match &n.kind {
        NodeKind::Group { children, .. } => children.iter().map(AsRef::as_ref).filter(|c| c.visible).collect(),
        _ => vec![],
    }
}

/// `props` over the properties `n` sets itself.
fn under(n: &Node, props: &StitchProps) -> StitchProps {
    n.stitch_props().map_or(*props, |p| p.inherit(props))
}

impl Walk<'_> {
    /// Sew converted lettering group `n` (the unit for it is the last one). Returns its children
    /// that are not pieces of a letter, to be sewn as objects of their own.
    pub(super) fn converted<'n>(&mut self, n: &'n Node, up: Under) -> Result<Vec<&'n Node>, String> {
        let mut letters: Vec<PieceLetter> = vec![];
        let mut others: Vec<&Node> = vec![];
        for (word, c) in children(n).into_iter().enumerate() {
            let props = under(c, &up.props);
            let is_word = children(c).iter().any(|l| matches!(l.kind, NodeKind::Group { .. }));
            if is_word {
                for l in children(c) {
                    self.letter(l, &under(l, &props), word, &mut letters, &mut others)?;
                }
            } else {
                self.letter(c, &props, word, &mut letters, &mut others)?;
            }
        }
        if letters.is_empty() {
            return Ok(others);
        }
        let params = super::text::lettering_params(&up.props, self.o);
        let budget = MAX_STEPS.saturating_sub(self.steps);
        let sewn = sew_pieces(&letters, &params, budget)?;
        self.part += 1;
        self.emit_letters(sewn, up)?;
        Ok(others)
    }

    /// Gather letter `l` (a group of pieces, or one piece) with properties `props` into `letters`;
    /// what is not a piece goes to `others`.
    fn letter<'n>(
        &mut self,
        l: &'n Node,
        props: &StitchProps,
        word: usize,
        letters: &mut Vec<PieceLetter>,
        others: &mut Vec<&'n Node>,
    ) -> Result<(), String> {
        let leaves = if matches!(l.kind, NodeKind::Group { .. }) { children(l) } else { vec![l] };
        let mut letter = PieceLetter { rgb: [0, 0, 0], word, columns: vec![], runs: vec![], fills: vec![] };
        let mut coloured = false;
        for leaf in leaves {
            let props = if std::ptr::eq(leaf, l) { *props } else { under(leaf, props) };
            match self.piece(leaf, &props, &mut letter)? {
                Some(rgb) if !coloured => {
                    letter.rgb = rgb;
                    coloured = true;
                }
                Some(_) => {}
                None => others.push(leaf),
            }
        }
        if !(letter.columns.is_empty() && letter.runs.is_empty() && letter.fills.is_empty()) {
            letters.push(letter);
        }
        Ok(())
    }

    /// Add `leaf` to `letter` when it is a piece: a two-rail satin column, a running stitch line
    /// or a tatami fill. Returns its thread, `None` when it is not a piece.
    fn piece(&mut self, leaf: &Node, props: &StitchProps, letter: &mut PieceLetter) -> Result<Option<[u8; 3]>, String> {
        if !matches!(leaf.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. }) {
            return Ok(None);
        }
        let stroke = leaf.appearance.stroke().filter(|s| s.visible && !s.paint.is_none()).map(|s| s.paint.clone());
        let fill = leaf.appearance.fill().filter(|f| f.visible && !f.paint.is_none()).map(|f| f.paint.clone());
        let mut subpaths = vec![];
        collect_subpaths(leaf, &mut subpaths);
        let budget = MAX_STEPS.saturating_sub(self.steps);
        match props.kind {
            StitchType::Satin => {
                let Some((a, b)) = two_rails(&subpaths) else { return Ok(None) };
                let Some(paint) = stroke.or(fill) else { return Ok(None) };
                let Some((left, right)) = paired_rails(a, b, budget)? else { return Ok(None) };
                let style = PieceStyle {
                    spacing: props.number(StitchParam::SatinSpacing).unwrap_or(self.o.stroke.spacing),
                    compensation: props.number(StitchParam::Compensation).unwrap_or(0.0),
                    underlay: props.underlay.unwrap_or(Underlay::None),
                };
                letter.columns.push((Column { left, right }, style));
                Ok(Some(thread(&paint, &mut self.patterned)))
            }
            StitchType::Run => {
                let [sp] = subpaths.as_slice() else { return Ok(None) };
                let Some(paint) = stroke.or(fill) else { return Ok(None) };
                let (mut pts, _) = flatten(sp, budget)?;
                if sp.closed
                    && let Some(&f) = pts.first()
                {
                    pts.push(f);
                }
                if pts.len() < 2 {
                    return Ok(None);
                }
                letter.runs.push(pts);
                Ok(Some(thread(&paint, &mut self.patterned)))
            }
            StitchType::Tatami => {
                let Some(paint) = fill.or(stroke) else { return Ok(None) };
                let mut rings: Vec<Vec<P>> = vec![];
                for sp in subpaths.iter().filter(|sp| sp.closed) {
                    let (pts, _) = flatten(sp, budget)?;
                    if pts.len() >= 3 {
                        rings.push(pts);
                    }
                }
                if rings.is_empty() {
                    return Ok(None);
                }
                letter.fills.push(rings);
                Ok(Some(thread(&paint, &mut self.patterned_fill)))
            }
            _ => Ok(None),
        }
    }
}
