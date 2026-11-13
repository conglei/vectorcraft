//! Type in a document sewn as lettering ([`crate::lettering`]): a text object's glyphs laid out as
//! the canvas draws them, in millimetres, with the words they belong to.

use vectorcraft_doc::{LetterTrims, Node, NodeKind, StitchParam, StitchProps, StitchType, TextObject};
use vectorcraft_geom::PathData;

use super::{FromDocOptions, flatten, thread};
use crate::fill::FillParams;
use crate::lettering::{
    COMPENSATION_MM, Letter, LetterMode, LetteringParams, MAX_LETTERS, MAX_SATIN_MM, MIN_COLUMN_MM, RUN_LENGTH_MM, RUN_UNDER_MM, SewnLetter,
    letter_height, mode_for, sew_letters,
};
use crate::{MAX_STEPS, UNITS_PER_MM};

/// The lettering settings of an object with (inherited) properties `props`: each it sets, else
/// the lettering defaults (satin spacing and fill settings from the document's options).
pub fn lettering_params(props: &StitchProps, o: &FromDocOptions) -> LetteringParams {
    let max_stitch = f64::from(o.max_stitch) / UNITS_PER_MM;
    let n = |p: StitchParam, default: f64| props.number(p).unwrap_or(default);
    let f = o.fill.params;
    LetteringParams {
        spacing: n(StitchParam::SatinSpacing, o.stroke.spacing),
        run_length: n(StitchParam::RunLength, RUN_LENGTH_MM),
        compensation: n(StitchParam::Compensation, COMPENSATION_MM),
        min_column: n(StitchParam::MinColumn, MIN_COLUMN_MM),
        run_under: n(StitchParam::RunUnder, RUN_UNDER_MM),
        max_satin: MAX_SATIN_MM,
        underlay: props.underlay.unwrap_or(true),
        trims: props.letter_trims.unwrap_or(LetterTrims::Word),
        max_stitch,
        fill: FillParams {
            angle: n(StitchParam::FillAngle, f.angle),
            spacing: n(StitchParam::FillSpacing, f.spacing),
            stitch_length: n(StitchParam::FillStitchLength, f.stitch_length),
            stagger: n(StitchParam::Stagger, f.stagger),
            max_stitch,
        },
    }
}

/// How type with stitch type `kind` is sewn: `None` when it isn't. Automatic, lettering and satin
/// type is lettering (satin, or running stitch when small); run type is running stitch along the
/// strokes; tatami type fills each letter; manual and none type is not sewn.
pub fn text_mode(kind: StitchType) -> Option<Option<LetterMode>> {
    match kind {
        StitchType::Auto | StitchType::Lettering | StitchType::Satin => Some(None),
        StitchType::Run => Some(Some(LetterMode::Run)),
        StitchType::Tatami => Some(Some(LetterMode::Fill)),
        StitchType::None | StitchType::Manual => None,
    }
}

/// Text object `t`'s letters in millimetres, in text order, each with its thread and word (words
/// are broken by white space and line ends). Glyphs drawn as images have no outline and are left
/// out. At most [`MAX_LETTERS`].
pub fn letters_of(t: &TextObject) -> Result<Vec<Letter>, String> {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let plain = t.plain_text();
    let mut out: Vec<Letter> = vec![];
    let mut word = 0usize;
    let mut last: Option<(usize, usize)> = None;
    let mut ignored = false;
    for g in &lay.glyphs {
        let own = plain.get(g.byte..g.byte.saturating_add(g.len)).unwrap_or("");
        if !own.is_empty() && own.chars().all(char::is_whitespace) {
            ignored = true;
            continue;
        }
        if let Some((end, line)) = last {
            let between = plain.get(end.min(g.byte)..g.byte.max(end)).unwrap_or("");
            if ignored || line != g.line || between.chars().any(char::is_whitespace) {
                word += 1;
            }
        }
        ignored = false;
        last = Some((g.byte.saturating_add(g.len), g.line));
        let path = PathData::from_bezpath(&g.outline).transformed(t.xf);
        let mut rings = vec![];
        for sp in &path.subpaths {
            let (pts, _) = flatten(sp, MAX_STEPS)?;
            if pts.len() >= 3 {
                rings.push(pts);
            }
        }
        if rings.is_empty() {
            continue;
        }
        if out.len() >= MAX_LETTERS {
            return Err(format!("lettering sews at most {MAX_LETTERS} letters"));
        }
        let style = t.runs.get(g.run).map(|r| r.style.clone()).unwrap_or_else(|| t.first_style());
        let appearance = style.appearance();
        let mut patterned = false;
        let rgb = appearance
            .fill()
            .filter(|f| f.visible && !f.paint.is_none())
            .map(|f| &f.paint)
            .or_else(|| appearance.stroke().filter(|s| s.visible && !s.paint.is_none()).map(|s| &s.paint))
            .map_or([0, 0, 0], |p| thread(p, &mut patterned));
        out.push(Letter { rings, rgb, word });
    }
    Ok(out)
}

/// Type sewn as lettering: the letters' mode, their height, and the letters sewn.
#[derive(Clone, Debug)]
pub struct Lettered {
    pub height_mm: f64,
    /// The mode chosen for the text (a letter may still be filled, see [`SewnLetter::mode`]).
    pub mode: LetterMode,
    pub params: LetteringParams,
    pub letters: Vec<SewnLetter>,
}

/// How text node `n` with (inherited) properties `props` is sewn by [`crate::from_document`] with
/// options `o`. `Ok(None)`: it isn't a text object, or its stitch type doesn't sew it.
pub fn lettering_of(n: &Node, props: &StitchProps, o: &FromDocOptions, budget: usize) -> Result<Option<Lettered>, String> {
    let NodeKind::Text(t) = &n.kind else { return Ok(None) };
    let Some(forced) = text_mode(props.kind) else { return Ok(None) };
    let letters = letters_of(t)?;
    let params = lettering_params(props, o);
    let height_mm = letter_height(&letters);
    let mode = forced.unwrap_or_else(|| mode_for(height_mm, &params));
    let letters = sew_letters(&letters, mode, &params, budget)?;
    Ok(Some(Lettered { height_mm, mode, params, letters }))
}
