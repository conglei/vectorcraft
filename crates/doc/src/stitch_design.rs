//! Embroidery › Sequence: the document-wide sewing settings: the order objects are sewn in and
//! where the design starts and ends. Saved in the native file; a file without them (every file
//! before they existed) sews in document order, bottom to top, as before.
//!
//! Each object's own entry and exit points are its stitch properties
//! ([`crate::StitchProps::entry`]).

use serde::{Deserialize, Serialize};
use vectorcraft_geom::Point;

use crate::NodeId;
use crate::stitch_props::lenient;

/// The most ids a saved order keeps: more than any document sews.
pub const MAX_ORDER: usize = 1_000_000;

/// The sewing order and the design's start and end points. Read leniently: a field of another
/// kind (a later version's, or damage) reads as unset and the rest of the document still opens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StitchDesign {
    /// The objects sewn first, in this order; every other sewn object follows in document order.
    /// Ids that name no sewn object (deleted since) are skipped, and an id listed twice counts
    /// once, where it is first listed.
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "lenient_order")]
    pub order: Vec<NodeId>,
    /// Where the machine starts, in document coordinates: the first object is entered nearest
    /// it, and a stitch file takes it as its origin. `None`: the export's `start` option decides.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub start: Option<Point>,
    /// Where the design should end, in document coordinates: the last object is left nearest it.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub end: Option<Point>,
    /// Embroidery › Hoop: the hoop the design is sewn in (`stitch.hoop.set`). `None`: none chosen.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub hoop: Option<HoopChoice>,
}

/// The largest hoop side kept, in mm.
pub const MAX_HOOP_MM: f64 = 2000.0;
/// The longest hoop name or id kept, in characters.
pub const MAX_HOOP_NAME: usize = 96;

/// The chosen hoop: a library hoop's id or `custom`, its name, inner size and shape, and where its
/// centre is on the page. Read leniently; [`HoopChoice::valid`] answers whether it can be used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HoopChoice {
    #[serde(deserialize_with = "lenient_string")]
    pub id: String,
    #[serde(deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(deserialize_with = "lenient_number")]
    pub width_mm: f64,
    #[serde(deserialize_with = "lenient_number")]
    pub height_mm: f64,
    /// `rect` or `round`; anything else reads as `rect`.
    #[serde(deserialize_with = "lenient_string")]
    pub shape: String,
    /// A cap frame.
    #[serde(skip_serializing_if = "std::ops::Not::not", deserialize_with = "lenient_bool")]
    pub cap: bool,
    /// The hoop's centre in document coordinates. `None`: the first artboard's centre.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub center: Option<Point>,
}

fn lenient_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let mut s: String = lenient(d)?.unwrap_or_default();
    if let Some((i, _)) = s.char_indices().nth(MAX_HOOP_NAME) {
        s.truncate(i);
    }
    Ok(s)
}

fn lenient_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(lenient(d)?.unwrap_or(0.0))
}

fn lenient_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(lenient(d)?.unwrap_or(false))
}

impl HoopChoice {
    /// Whether the size is finite and positive, at most [`MAX_HOOP_MM`], and the centre (if any)
    /// finite: a damaged choice is ignored rather than used.
    pub fn valid(&self) -> bool {
        let side = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_HOOP_MM;
        side(self.width_mm) && side(self.height_mm) && self.center.is_none_or(|p| p.x.is_finite() && p.y.is_finite())
    }
}

fn lenient_order<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<NodeId>, D::Error> {
    let mut v: Vec<NodeId> = lenient(d)?.unwrap_or_default();
    v.truncate(MAX_ORDER);
    Ok(v)
}

impl StitchDesign {
    /// Nothing set: sew in document order, start and end as the export says.
    pub fn is_empty(&self) -> bool {
        *self == StitchDesign::default()
    }

    /// The start point, if set and finite.
    pub fn start_point(&self) -> Option<Point> {
        self.start.filter(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// The end point, if set and finite.
    pub fn end_point(&self) -> Option<Point> {
        self.end.filter(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// The chosen hoop, if set and valid.
    pub fn hoop(&self) -> Option<&HoopChoice> {
        self.hoop.as_ref().filter(|h| h.valid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_reads_leniently() {
        let d = StitchDesign { order: vec![NodeId(3), NodeId(1)], start: Some(Point::new(1.0, 2.0)), end: None, hoop: None };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["order"], serde_json::json!([3, 1]));
        assert!(v.get("end").is_none());
        assert_eq!(serde_json::from_value::<StitchDesign>(v).unwrap(), d);
        let back: StitchDesign =
            serde_json::from_value(serde_json::json!({"order": "x", "start": 5, "end": {"x": 1.0, "y": 1.0}, "later": 1})).unwrap();
        assert_eq!(back.order, vec![]);
        assert_eq!(back.start, None);
        assert!(back.end.is_some());
        assert!(StitchDesign::default().is_empty());
        assert!(serde_json::to_value(StitchDesign::default()).unwrap().as_object().unwrap().is_empty());
    }

    #[test]
    fn a_hoop_round_trips_and_damage_reads_as_unusable() {
        let h = HoopChoice {
            id: "square-100".into(),
            name: "Square".into(),
            width_mm: 100.0,
            height_mm: 100.0,
            shape: "rect".into(),
            cap: false,
            center: None,
        };
        let d = StitchDesign { hoop: Some(h.clone()), ..Default::default() };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["hoop"]["widthMm"], 100.0);
        assert!(v["hoop"].get("cap").is_none());
        assert_eq!(serde_json::from_value::<StitchDesign>(v).unwrap().hoop(), Some(&h));
        let bad: StitchDesign = serde_json::from_value(serde_json::json!({"hoop": {"widthMm": "x", "heightMm": 1e9, "name": 5}})).unwrap();
        assert!(bad.hoop.is_some() && bad.hoop().is_none());
        let junk: StitchDesign = serde_json::from_value(serde_json::json!({"hoop": 7})).unwrap();
        assert_eq!(junk.hoop, None);
        let long: StitchDesign =
            serde_json::from_value(serde_json::json!({"hoop": {"name": "n".repeat(500), "widthMm": 10, "heightMm": 10}})).unwrap();
        assert_eq!(long.hoop().unwrap().name.len(), MAX_HOOP_NAME);
    }
}
