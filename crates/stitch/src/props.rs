//! What an object sews with, for the Stitch Properties panel and `stitch.props`: its stitch
//! properties over those of the groups, compound path and layer holding it, then the export's
//! options ([`FromDocOptions`]), as [`crate::from_document`] uses them.

use vectorcraft_doc::{Document, MAX_DEPTH, Node, NodeId, NodeKind, StitchParam, StitchProps, StitchType};

use crate::from_doc::{FillType, FromDocOptions, Mode, StrokeType};
use crate::to_doc::is_marked;

/// The stitch properties object `id` sews with, set on it or inherited, field by field, from what
/// holds it (the export's options not yet applied), and whether it lies on a layer an opened
/// stitch file made (where they are not used). `None`: no such object.
pub fn inherited_props(doc: &Document, id: NodeId) -> Option<(StitchProps, bool)> {
    let mut chain: Vec<&Node> = vec![];
    let mut at = Some(id);
    while let Some(i) = at {
        // Documents are at most MAX_DEPTH deep; a longer chain is damage, cut short.
        if chain.len() > MAX_DEPTH {
            break;
        }
        chain.push(doc.node(i)?);
        at = doc.parent_of(i);
    }
    let mut props = StitchProps::default();
    let mut marked = false;
    for n in chain.iter().rev() {
        props = n.stitch_props().map_or(props, |p| p.inherit(&props));
        marked |= is_marked(n);
    }
    Some((props, marked))
}

/// The value of number `p` an object with (inherited) properties `props` sews with: its own, else
/// the options'. `None`: none applies (a satin width, which is then the stroke weight; and
/// compensation, not yet used).
pub fn effective_number(props: &StitchProps, p: StitchParam, o: &FromDocOptions) -> Option<f64> {
    props.number(p).or(match p {
        StitchParam::FillAngle => Some(o.fill.params.angle),
        StitchParam::FillSpacing => Some(o.fill.params.spacing),
        StitchParam::FillStitchLength => Some(o.fill.params.stitch_length),
        StitchParam::Stagger => Some(o.fill.params.stagger),
        StitchParam::SatinSpacing => Some(o.stroke.spacing),
        StitchParam::SatinWidth => o.stroke.width,
        StitchParam::RunLength => Some(f64::from(o.stitch_length) / crate::UNITS_PER_MM),
        StitchParam::Compensation => None,
    })
}

/// How [`crate::from_document`] sews path or compound path `n` with (inherited) properties
/// `props`, `marked` when on an opened stitch file's layer: the stitch types in sewing order (an
/// automatic object's fill, then its outline), empty for nothing sewn or for an object of
/// another kind.
pub fn sewn_as(n: &Node, props: &StitchProps, marked: bool, o: &FromDocOptions) -> Vec<StitchType> {
    if !matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. }) {
        return vec![];
    }
    let stroke = n.appearance.stroke().is_some_and(|s| s.visible && !s.paint.is_none());
    let fill = n.appearance.fill().is_some_and(|f| f.visible && !f.paint.is_none());
    let mode = o.mode.unwrap_or(if marked { Mode::Manual } else { Mode::Run });
    let outline = if o.stroke.kind == StrokeType::Satin && !marked {
        StitchType::Satin
    } else if mode == Mode::Manual {
        StitchType::Manual
    } else {
        StitchType::Run
    };
    let kind = if marked { StitchType::Auto } else { props.kind };
    match kind {
        StitchType::None => vec![],
        StitchType::Auto => {
            let mut v = vec![];
            if fill && !marked && o.fill.kind == FillType::Tatami {
                v.push(StitchType::Tatami);
            }
            if stroke {
                v.push(outline);
            }
            v
        }
        t if fill || stroke => vec![t],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, LayerColor};
    use vectorcraft_geom::{PathData, Point, SubPath};

    use super::*;
    use crate::{StrokeOptions, from_document};

    fn square(id: u64, fill: bool, stroke: bool, props: StitchProps) -> Node {
        let path =
            PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(30.0, 30.0), Point::new(0.0, 30.0)], true));
        let c = |on: bool, rgb: (u8, u8, u8)| if on { Paint::solid(Color::rgb8(rgb.0, rgb.1, rgb.2)) } else { Paint::None };
        let mut n = Node::path(NodeId(id), path, Appearance::basic(c(fill, (0, 0, 255)), c(stroke, (255, 0, 0)), 1.0));
        n.set_stitch_props(props);
        n
    }

    fn doc(group_props: StitchProps, art: Vec<Node>) -> Document {
        let mut d = Document::new(200.0, 200.0);
        let mut g = Node::group(NodeId(50), art.into_iter().map(Arc::new).collect());
        g.set_stitch_props(group_props);
        let mut layer = Node::layer(NodeId(60), "Art", LayerColor::Preset(0));
        if let Some(c) = layer.children_mut() {
            *c = vec![Arc::new(g)];
        }
        d.layers = vec![Arc::new(layer)];
        d
    }

    #[test]
    fn properties_are_inherited_and_the_options_fill_the_rest() {
        let group = StitchProps { kind: StitchType::Satin, satin_width: Some(4.0), ..Default::default() };
        let d = doc(group, vec![square(1, true, true, StitchProps { satin_spacing: Some(0.6), ..Default::default() })]);
        let (p, marked) = inherited_props(&d, NodeId(1)).unwrap();
        assert!(!marked);
        assert_eq!((p.kind, p.satin_width, p.satin_spacing), (StitchType::Satin, Some(4.0), Some(0.6)));
        let o = FromDocOptions::default();
        assert_eq!(effective_number(&p, StitchParam::FillSpacing, &o), Some(0.4));
        assert_eq!(effective_number(&p, StitchParam::RunLength, &o), Some(2.5));
        assert_eq!(effective_number(&p, StitchParam::SatinWidth, &o), Some(4.0));
        assert_eq!(effective_number(&StitchProps::default(), StitchParam::SatinWidth, &o), None);
        assert!(inherited_props(&d, NodeId(999)).is_none());
    }

    /// What `sewn_as` says matches the thread blocks `from_document` makes.
    #[test]
    fn sewn_as_matches_the_plan() {
        let satin_doc = FromDocOptions { stroke: StrokeOptions { kind: StrokeType::Satin, ..Default::default() }, ..Default::default() };
        for o in [FromDocOptions::default(), satin_doc] {
            for kind in StitchType::ALL {
                for (fill, stroke) in [(true, true), (true, false), (false, true)] {
                    let n = square(1, fill, stroke, StitchProps { kind, ..Default::default() });
                    let d = doc(StitchProps::default(), vec![n.clone()]);
                    let (props, marked) = inherited_props(&d, NodeId(1)).unwrap();
                    let sews = sewn_as(&n, &props, marked, &o);
                    let blocks = from_document(&d, &o).map_or(0, |p| p.plan.blocks.len());
                    assert_eq!(sews.len(), blocks, "{kind:?} fill {fill} stroke {stroke}: {sews:?}");
                }
            }
        }
    }
}
