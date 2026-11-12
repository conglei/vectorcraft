//! Embroidery › Stitch Properties: how one object is sewn, and the parameters it sews with.
//!
//! An object's [`StitchProps`] are stored on the node ([`crate::NodeExtras::stitch`]) and saved in
//! the native file. Stitches are not stored: an export makes them from the object's outline, so
//! moving, scaling or reshaping an object makes its stitches again at the same density.
//!
//! What an object sews with is found field by field: the object's own value, else the value of the
//! nearest group, layer or compound path holding it that sets one ([`StitchProps::inherit`]), else
//! the document default of the export's stitch options, else the built-in default. Values are read
//! from files, so a value outside its [`StitchParam::range`] is ignored where it is used
//! ([`StitchProps::number`]).

use serde::{Deserialize, Serialize};

use crate::Node;

/// How an object becomes stitches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StitchType {
    /// As the document's stitch options say: a fill is sewn as tatami, then a stroke as running
    /// stitch (or satin, or anchor by anchor, by those options).
    #[default]
    Auto,
    /// Not sewn.
    None,
    /// Running stitch along the outline, in the stroke colour (the fill colour without a stroke).
    Run,
    /// A satin column centred on the path, in the stroke colour (the fill colour without one).
    Satin,
    /// Tatami fill of the region the outline encloses, in the fill colour (the stroke colour
    /// without a fill).
    Tatami,
    /// One needle point per anchor, in the stroke colour (the fill colour without one).
    Manual,
    /// Type sewn as lettering: satin columns along each letter's strokes (running stitch for
    /// small letters, tatami for strokes too wide for satin), in the characters' colour. Type
    /// that is automatic is sewn this way too; on a path it sews as automatic.
    Lettering,
}

impl StitchType {
    /// Every type, in the order the Stitch Properties panel lists them.
    pub const ALL: [StitchType; 7] =
        [StitchType::Auto, StitchType::None, StitchType::Run, StitchType::Satin, StitchType::Tatami, StitchType::Manual, StitchType::Lettering];

    /// The type's id in commands and files.
    pub fn id(self) -> &'static str {
        match self {
            StitchType::Auto => "auto",
            StitchType::None => "none",
            StitchType::Run => "run",
            StitchType::Satin => "satin",
            StitchType::Tatami => "tatami",
            StitchType::Manual => "manual",
            StitchType::Lettering => "lettering",
        }
    }

    /// The type named `id` ([`StitchType::id`]).
    pub fn parse(id: &str) -> Option<StitchType> {
        StitchType::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The type's name in the panel.
    pub fn label(self) -> &'static str {
        match self {
            StitchType::Auto => "Automatic",
            StitchType::None => "Not sewn",
            StitchType::Run => "Running stitch",
            StitchType::Satin => "Satin column",
            StitchType::Tatami => "Tatami fill",
            StitchType::Manual => "Manual (one point per anchor)",
            StitchType::Lettering => "Lettering (type)",
        }
    }
}

/// Where lettering trims the thread between letters ([`StitchProps::letter_trims`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LetterTrims {
    /// Between words and lines; letters of a word are joined by a jump (or a stitch when close).
    #[default]
    Word,
    /// Between every two letters.
    Letter,
    /// Never: letters and words are joined by jumps.
    None,
}

impl LetterTrims {
    pub const ALL: [LetterTrims; 3] = [LetterTrims::Word, LetterTrims::Letter, LetterTrims::None];

    /// The id in commands and files.
    pub fn id(self) -> &'static str {
        match self {
            LetterTrims::Word => "word",
            LetterTrims::Letter => "letter",
            LetterTrims::None => "none",
        }
    }

    /// The value named `id` ([`LetterTrims::id`]).
    pub fn parse(id: &str) -> Option<LetterTrims> {
        LetterTrims::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The name in the panel.
    pub fn label(self) -> &'static str {
        match self {
            LetterTrims::Word => "Per word",
            LetterTrims::Letter => "Per letter",
            LetterTrims::None => "None",
        }
    }
}

/// A number an object may set ([`StitchProps::number`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StitchParam {
    /// Tatami row direction, degrees counter-clockwise from horizontal.
    FillAngle,
    /// Tatami row spacing, mm.
    FillSpacing,
    /// Tatami stitch length along a row, mm.
    FillStitchLength,
    /// Tatami row offset, as a fraction of the stitch length.
    Stagger,
    /// Satin spacing, peak to peak, mm.
    SatinSpacing,
    /// Satin width, rail to rail, mm (default: the stroke weight).
    SatinWidth,
    /// Running stitch length, mm.
    RunLength,
    /// Pull compensation, mm each side: each satin needle point is pushed this far outward (two-rail
    /// satin columns and lettering).
    Compensation,
    /// Lettering: the narrowest satin column, mm; narrower strokes are sewn as running stitch.
    MinColumn,
    /// Lettering: letters shorter than this, mm, are sewn as running stitch along their strokes.
    RunUnder,
}

impl StitchParam {
    /// Every number, in the order commands list them.
    pub const ALL: [StitchParam; 10] = [
        StitchParam::FillAngle,
        StitchParam::FillSpacing,
        StitchParam::FillStitchLength,
        StitchParam::Stagger,
        StitchParam::SatinSpacing,
        StitchParam::SatinWidth,
        StitchParam::RunLength,
        StitchParam::Compensation,
        StitchParam::MinColumn,
        StitchParam::RunUnder,
    ];

    /// The number's key in commands and files.
    pub fn key(self) -> &'static str {
        match self {
            StitchParam::FillAngle => "fillAngle",
            StitchParam::FillSpacing => "fillSpacing",
            StitchParam::FillStitchLength => "fillStitchLength",
            StitchParam::Stagger => "stagger",
            StitchParam::SatinSpacing => "satinSpacing",
            StitchParam::SatinWidth => "satinWidth",
            StitchParam::RunLength => "runLength",
            StitchParam::Compensation => "compensation",
            StitchParam::MinColumn => "minColumn",
            StitchParam::RunUnder => "runUnder",
        }
    }

    /// The number keyed `key` ([`StitchParam::key`]).
    pub fn parse(key: &str) -> Option<StitchParam> {
        StitchParam::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The values the number takes (the ranges of the export's stitch options).
    pub fn range(self) -> (f64, f64) {
        match self {
            StitchParam::FillAngle => (-360.0, 360.0),
            StitchParam::FillSpacing | StitchParam::SatinSpacing => (0.2, 5.0),
            StitchParam::FillStitchLength => (1.0, 8.0),
            StitchParam::Stagger => (0.0, 1.0),
            StitchParam::SatinWidth => (0.5, 20.0),
            StitchParam::RunLength => (0.5, 12.0),
            StitchParam::Compensation => (0.0, 2.0),
            StitchParam::MinColumn => (0.3, 3.0),
            StitchParam::RunUnder => (0.0, 20.0),
        }
    }

    /// The number's unit: `mm`, `degrees` or empty (a fraction).
    pub fn unit(self) -> &'static str {
        match self {
            StitchParam::FillAngle => "degrees",
            StitchParam::Stagger => "",
            _ => "mm",
        }
    }

    /// The stitch types that use the number (with [`StitchType::Auto`] standing for every type
    /// the document's options may choose).
    pub fn used_by(self) -> &'static [StitchType] {
        match self {
            StitchParam::FillAngle | StitchParam::FillSpacing | StitchParam::FillStitchLength | StitchParam::Stagger => {
                &[StitchType::Auto, StitchType::Tatami]
            }
            StitchParam::SatinSpacing => &[StitchType::Auto, StitchType::Satin, StitchType::Lettering],
            StitchParam::SatinWidth => &[StitchType::Auto, StitchType::Satin],
            StitchParam::RunLength => &[StitchType::Auto, StitchType::Run, StitchType::Lettering],
            StitchParam::Compensation => &[StitchType::Auto, StitchType::Satin, StitchType::Lettering],
            StitchParam::MinColumn | StitchParam::RunUnder => &[StitchType::Lettering],
        }
    }

    /// `v`, if it is finite and in [`StitchParam::range`].
    pub fn checked(self, v: f64) -> Option<f64> {
        let (lo, hi) = self.range();
        (v.is_finite() && (lo..=hi).contains(&v)).then_some(v)
    }
}

/// How one object is sewn (Embroidery › Stitch Properties). Each field left unset is inherited
/// ([`StitchProps::inherit`]); all unset is the same as none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StitchProps {
    /// The stitch type; [`StitchType::Auto`] inherits.
    #[serde(rename = "type", skip_serializing_if = "is_auto", deserialize_with = "lenient_type")]
    pub kind: StitchType,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub fill_angle: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub fill_spacing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub fill_stitch_length: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub stagger: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub satin_spacing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub satin_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub run_length: Option<f64>,
    /// Underlay: a centre run under two-rail satin columns and satin lettering (on by default for
    /// lettering); not yet sewn under fills or single-path satin.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub underlay: Option<bool>,
    /// Pull compensation in mm each side ([`StitchParam::Compensation`]).
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub compensation: Option<f64>,
    /// Lettering: the narrowest satin column in mm ([`StitchParam::MinColumn`]).
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub min_column: Option<f64>,
    /// Lettering: the letter height in mm under which letters are running stitch
    /// ([`StitchParam::RunUnder`]).
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub run_under: Option<f64>,
    /// Lettering: where the thread is trimmed between letters.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub letter_trims: Option<LetterTrims>,
    /// Embroidery › Sequence: where sewing enters the object, as a fraction 0–1 of the way along
    /// its outline (its subpaths one after another, by arc length), so the point moves with the
    /// object when it is moved, scaled or reshaped. Only the object's own: never inherited.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub entry: Option<f64>,
    /// Where sewing leaves the object, as for [`StitchProps::entry`].
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub exit: Option<f64>,
    /// The entry point Embroidery › Closest Join chose, as for [`StitchProps::entry`]: used when
    /// [`StitchProps::entry`] is unset, and chosen again by the next closest join (which keeps
    /// the points the user set).
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub join_entry: Option<f64>,
    /// The exit point Closest Join chose, as for [`StitchProps::join_entry`].
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub join_exit: Option<f64>,
}

/// A fraction along an outline, if it is finite and within 0–1 (a value out of range, from a
/// file, counts as unset).
fn fraction(v: Option<f64>) -> Option<f64> {
    v.filter(|t| t.is_finite() && (0.0..=1.0).contains(t))
}

fn is_auto(t: &StitchType) -> bool {
    *t == StitchType::Auto
}

/// A value read from a file: of the expected kind, or anything else (skipped).
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Ok(T),
    Other(serde::de::IgnoredAny),
}

/// A field read leniently: a value of another kind (a later version's, or damage) reads as unset,
/// so the rest of the document still opens.
pub(crate) fn lenient<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Ok(match Lenient::<T>::deserialize(d)? {
        Lenient::Ok(v) => Some(v),
        Lenient::Other(_) => None,
    })
}

/// The stitch type read leniently: a type this version doesn't know reads as automatic.
fn lenient_type<'de, D: serde::Deserializer<'de>>(d: D) -> Result<StitchType, D::Error> {
    Ok(lenient::<D, String>(d)?.as_deref().and_then(StitchType::parse).unwrap_or_default())
}

impl StitchProps {
    /// The entry point set here ([`StitchProps::entry`]), if it is in range.
    pub fn entry_at(&self) -> Option<f64> {
        fraction(self.entry)
    }

    /// The exit point set here ([`StitchProps::exit`]), if it is in range.
    pub fn exit_at(&self) -> Option<f64> {
        fraction(self.exit)
    }

    /// The entry point Closest Join chose ([`StitchProps::join_entry`]), if it is in range.
    pub fn join_entry_at(&self) -> Option<f64> {
        fraction(self.join_entry)
    }

    /// The exit point Closest Join chose ([`StitchProps::join_exit`]), if it is in range.
    pub fn join_exit_at(&self) -> Option<f64> {
        fraction(self.join_exit)
    }

    /// Where the object is entered: the user's point, else the one Closest Join chose.
    pub fn sewn_entry(&self) -> Option<f64> {
        self.entry_at().or(fraction(self.join_entry))
    }

    /// Where the object is left: the user's point, else the one Closest Join chose.
    pub fn sewn_exit(&self) -> Option<f64> {
        self.exit_at().or(fraction(self.join_exit))
    }

    /// Nothing set.
    pub fn is_empty(&self) -> bool {
        *self == StitchProps::default()
    }

    fn slot(&self, p: StitchParam) -> Option<f64> {
        match p {
            StitchParam::FillAngle => self.fill_angle,
            StitchParam::FillSpacing => self.fill_spacing,
            StitchParam::FillStitchLength => self.fill_stitch_length,
            StitchParam::Stagger => self.stagger,
            StitchParam::SatinSpacing => self.satin_spacing,
            StitchParam::SatinWidth => self.satin_width,
            StitchParam::RunLength => self.run_length,
            StitchParam::Compensation => self.compensation,
            StitchParam::MinColumn => self.min_column,
            StitchParam::RunUnder => self.run_under,
        }
    }

    /// The number `p` set here, if it is in range (a value out of range, from a file, counts as
    /// unset).
    pub fn number(&self, p: StitchParam) -> Option<f64> {
        self.slot(p).and_then(|v| p.checked(v))
    }

    /// Set (or, with `None`, unset) the number `p`. The caller checks the range.
    pub fn set_number(&mut self, p: StitchParam, v: Option<f64>) {
        let slot = match p {
            StitchParam::FillAngle => &mut self.fill_angle,
            StitchParam::FillSpacing => &mut self.fill_spacing,
            StitchParam::FillStitchLength => &mut self.fill_stitch_length,
            StitchParam::Stagger => &mut self.stagger,
            StitchParam::SatinSpacing => &mut self.satin_spacing,
            StitchParam::SatinWidth => &mut self.satin_width,
            StitchParam::RunLength => &mut self.run_length,
            StitchParam::Compensation => &mut self.compensation,
            StitchParam::MinColumn => &mut self.min_column,
            StitchParam::RunUnder => &mut self.run_under,
        };
        *slot = v;
    }

    /// These properties over `outer`, the properties of what holds the object: each field unset
    /// here (an [`StitchType::Auto`] type, a `None` value, or a value out of range) takes
    /// `outer`'s. The entry and exit points are the object's own and are left out.
    pub fn inherit(&self, outer: &StitchProps) -> StitchProps {
        let mut out = StitchProps {
            kind: if self.kind == StitchType::Auto { outer.kind } else { self.kind },
            underlay: self.underlay.or(outer.underlay),
            letter_trims: self.letter_trims.or(outer.letter_trims),
            ..StitchProps::default()
        };
        for p in StitchParam::ALL {
            out.set_number(p, self.number(p).or_else(|| outer.number(p)));
        }
        out
    }
}

impl Node {
    /// The object's own stitch properties, if it has any (Embroidery › Stitch Properties).
    pub fn stitch_props(&self) -> Option<&StitchProps> {
        self.extras().stitch.as_deref()
    }

    /// Set the object's own stitch properties; empty ones are removed.
    pub fn set_stitch_props(&mut self, props: StitchProps) {
        self.extras_mut().stitch = (!props.is_empty()).then(|| Box::new(props));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_keys_parse_back() {
        for t in StitchType::ALL {
            assert_eq!(StitchType::parse(t.id()), Some(t));
        }
        for p in StitchParam::ALL {
            assert_eq!(StitchParam::parse(p.key()), Some(p));
        }
        assert_eq!(StitchType::parse("zigzag"), None);
    }

    #[test]
    fn inheritance_takes_each_unset_field_from_outside() {
        let outer = StitchProps { kind: StitchType::Satin, satin_width: Some(3.0), run_length: Some(2.0), ..Default::default() };
        let own = StitchProps { run_length: Some(1.5), ..Default::default() };
        let got = own.inherit(&outer);
        assert_eq!(got.kind, StitchType::Satin);
        assert_eq!((got.satin_width, got.run_length), (Some(3.0), Some(1.5)));
        let own = StitchProps { kind: StitchType::Run, ..Default::default() };
        assert_eq!(own.inherit(&outer).kind, StitchType::Run);
    }

    #[test]
    fn out_of_range_values_count_as_unset() {
        let own = StitchProps { fill_spacing: Some(0.0), fill_angle: Some(f64::NAN), ..Default::default() };
        assert_eq!(own.number(StitchParam::FillSpacing), None);
        assert_eq!(own.number(StitchParam::FillAngle), None);
        let outer = StitchProps { fill_spacing: Some(0.6), ..Default::default() };
        assert_eq!(own.inherit(&outer).fill_spacing, Some(0.6));
    }

    #[test]
    fn serialized_with_only_what_is_set() {
        let p = StitchProps { kind: StitchType::Tatami, fill_angle: Some(45.0), ..Default::default() };
        assert_eq!(serde_json::to_value(p).unwrap(), serde_json::json!({"type": "tatami", "fillAngle": 45.0}));
        let back: StitchProps = serde_json::from_value(serde_json::json!({"type": "tatami", "fillAngle": 45.0, "later": 1})).unwrap();
        assert_eq!(back, p);
        assert!(serde_json::from_value::<StitchProps>(serde_json::json!({})).unwrap().is_empty());
    }

    #[test]
    fn values_of_another_kind_read_as_unset() {
        let back: StitchProps =
            serde_json::from_value(serde_json::json!({"type": "zigzag", "fillAngle": "steep", "runLength": 2.0, "underlay": [1]})).unwrap();
        assert_eq!(back, StitchProps { run_length: Some(2.0), ..Default::default() });
        let back: StitchProps = serde_json::from_value(serde_json::json!({"type": 7, "satinWidth": {"a": 1}})).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn entry_and_exit_are_fractions_kept_out_of_inheritance() {
        let own = StitchProps { entry: Some(0.25), exit: Some(1.5), ..Default::default() };
        assert_eq!((own.entry_at(), own.exit_at()), (Some(0.25), None));
        assert!(!own.is_empty());
        let inner = StitchProps::default().inherit(&own);
        assert_eq!((inner.entry, inner.exit), (None, None));
        let back: StitchProps = serde_json::from_value(serde_json::json!({"entry": 0.5, "exit": "end"})).unwrap();
        assert_eq!((back.entry, back.exit), (Some(0.5), None));
        let joined = StitchProps { entry: Some(0.1), join_entry: Some(0.9), join_exit: Some(0.4), ..Default::default() };
        assert_eq!((joined.sewn_entry(), joined.sewn_exit()), (Some(0.1), Some(0.4)), "the user's point wins");
        assert_eq!(serde_json::to_value(joined).unwrap(), serde_json::json!({"entry": 0.1, "joinEntry": 0.9, "joinExit": 0.4}));
    }

    #[test]
    fn lettering_fields_round_trip_and_inherit() {
        let p = StitchProps {
            kind: StitchType::Lettering,
            min_column: Some(1.2),
            run_under: Some(4.0),
            letter_trims: Some(LetterTrims::Letter),
            ..Default::default()
        };
        let v = serde_json::to_value(p).unwrap();
        assert_eq!(v, serde_json::json!({"type": "lettering", "minColumn": 1.2, "runUnder": 4.0, "letterTrims": "letter"}));
        assert_eq!(serde_json::from_value::<StitchProps>(v).unwrap(), p);
        let inner = StitchProps::default().inherit(&p);
        assert_eq!((inner.kind, inner.letter_trims, inner.min_column), (StitchType::Lettering, Some(LetterTrims::Letter), Some(1.2)));
        let odd: StitchProps = serde_json::from_value(serde_json::json!({"letterTrims": "sometimes", "minColumn": 9.0})).unwrap();
        assert_eq!((odd.letter_trims, odd.number(StitchParam::MinColumn)), (None, None));
        for t in LetterTrims::ALL {
            assert_eq!(LetterTrims::parse(t.id()), Some(t));
        }
    }

    #[test]
    fn empty_props_are_removed_from_the_node() {
        let mut n = Node::group(crate::NodeId(1), vec![]);
        n.set_stitch_props(StitchProps { kind: StitchType::Run, ..Default::default() });
        assert_eq!(n.stitch_props().map(|p| p.kind), Some(StitchType::Run));
        n.set_stitch_props(StitchProps::default());
        assert!(n.stitch_props().is_none());
        assert!(n.extras_box().is_none());
    }
}
