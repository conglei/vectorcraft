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
}

impl StitchType {
    /// Every type, in the order the Stitch Properties panel lists them.
    pub const ALL: [StitchType; 6] = [StitchType::Auto, StitchType::None, StitchType::Run, StitchType::Satin, StitchType::Tatami, StitchType::Manual];

    /// The type's id in commands and files.
    pub fn id(self) -> &'static str {
        match self {
            StitchType::Auto => "auto",
            StitchType::None => "none",
            StitchType::Run => "run",
            StitchType::Satin => "satin",
            StitchType::Tatami => "tatami",
            StitchType::Manual => "manual",
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
    /// Pull compensation, mm: kept and saved, not yet used by the stitch generators.
    Compensation,
}

impl StitchParam {
    /// Every number, in the order commands list them.
    pub const ALL: [StitchParam; 8] = [
        StitchParam::FillAngle,
        StitchParam::FillSpacing,
        StitchParam::FillStitchLength,
        StitchParam::Stagger,
        StitchParam::SatinSpacing,
        StitchParam::SatinWidth,
        StitchParam::RunLength,
        StitchParam::Compensation,
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
            StitchParam::SatinSpacing | StitchParam::SatinWidth => &[StitchType::Auto, StitchType::Satin],
            StitchParam::RunLength => &[StitchType::Auto, StitchType::Run],
            StitchParam::Compensation => &[StitchType::Auto, StitchType::Satin, StitchType::Tatami],
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
    /// Underlay under satin and fills: kept and saved, not yet sewn.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub underlay: Option<bool>,
    /// Pull compensation in mm: kept and saved, not yet used.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub compensation: Option<f64>,
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
        };
        *slot = v;
    }

    /// These properties over `outer`, the properties of what holds the object: each field unset
    /// here (an [`StitchType::Auto`] type, a `None` value, or a value out of range) takes
    /// `outer`'s.
    pub fn inherit(&self, outer: &StitchProps) -> StitchProps {
        let mut out = StitchProps {
            kind: if self.kind == StitchType::Auto { outer.kind } else { self.kind },
            underlay: self.underlay.or(outer.underlay),
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
    fn empty_props_are_removed_from_the_node() {
        let mut n = Node::group(crate::NodeId(1), vec![]);
        n.set_stitch_props(StitchProps { kind: StitchType::Run, ..Default::default() });
        assert_eq!(n.stitch_props().map(|p| p.kind), Some(StitchType::Run));
        n.set_stitch_props(StitchProps::default());
        assert!(n.stitch_props().is_none());
        assert!(n.extras_box().is_none());
    }
}
