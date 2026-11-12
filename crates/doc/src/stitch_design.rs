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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_reads_leniently() {
        let d = StitchDesign { order: vec![NodeId(3), NodeId(1)], start: Some(Point::new(1.0, 2.0)), end: None };
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
}
