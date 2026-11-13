//! Embroidery › Sequence: the document-wide sewing settings: the order objects are sewn in and
//! where the design starts and ends, the hoop it is sewn in and the fabric it is sewn on. Saved in
//! the native file; a file without them (every file before they existed) sews in document order,
//! bottom to top, as before, on the default fabric.
//!
//! Each object's own entry and exit points are its stitch properties
//! ([`crate::StitchProps::entry`]).

use serde::{Deserialize, Serialize};
use vectorcraft_geom::Point;

use crate::NodeId;
use crate::stitch_props::lenient;

/// The most ids a saved order keeps: more than any document sews.
pub const MAX_ORDER: usize = 1_000_000;

/// The sewing order, the design's start and end points, its hoop and its fabric. Read leniently: a field of
/// another kind (a later version's, or damage) reads as unset and the rest of the document still
/// opens.
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
    pub hoop: Option<Hoop>,
    /// The fabric colour the stitches are previewed on (`stitch.setup {fabric}`), as `#rrggbb` in
    /// the file. `None`: [`DEFAULT_FABRIC`]. A value that is not such a colour reads as `None`.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "hex_out", deserialize_with = "hex_in")]
    pub fabric: Option<[u8; 3]>,
}

/// The default fabric: a light, warm neutral.
pub const DEFAULT_FABRIC: [u8; 3] = [236, 232, 222];

/// `rgb` as `#rrggbb`.
pub fn rgb_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour `#rrggbb` (or `rrggbb`) names.
pub fn parse_rgb_hex(v: &str) -> Option<[u8; 3]> {
    let h = v.strip_prefix('#').unwrap_or(v);
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn hex_out<S: serde::Serializer>(v: &Option<[u8; 3]>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(c) => s.serialize_str(&rgb_hex(*c)),
        None => s.serialize_none(),
    }
}

fn hex_in<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<[u8; 3]>, D::Error> {
    Ok(lenient::<D, String>(d)?.as_deref().and_then(parse_rgb_hex))
}

/// The largest hoop side kept, in mm.
pub const MAX_HOOP_MM: f64 = 2000.0;
/// The longest hoop name or id kept, in characters.
pub const MAX_HOOP_NAME: usize = 96;

/// A hoop's outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoopShape {
    /// A rectangle (a rounded rectangle's sewable area is taken as the full rectangle).
    #[default]
    Rectangle,
    /// A circle or oval: the sewable area is the ellipse inside width × height.
    Round,
}

impl HoopShape {
    /// The id used in files and commands: `rectangle` or `round`.
    pub fn id(self) -> &'static str {
        match self {
            HoopShape::Rectangle => "rectangle",
            HoopShape::Round => "round",
        }
    }

    /// The shape an id names (`rectangle` or `rect`; `round`, `oval` or `circle`).
    pub fn parse(s: &str) -> Option<HoopShape> {
        match s {
            "rectangle" | "rect" => Some(HoopShape::Rectangle),
            "round" | "oval" | "circle" => Some(HoopShape::Round),
            _ => None,
        }
    }
}

/// A hoop or frame: its library id (`None` for a custom hoop), name, inner (sewable) size in mm
/// with the width along the machine's x axis, shape, and whether it is a cap frame. The design is
/// sewn centred in it: `stitch.hoop.fit` reports how far the design's centre is from the hoop's
/// centre on the page (the first artboard's centre). Read leniently; [`Hoop::valid`] answers
/// whether it can be used.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Hoop {
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient_id")]
    pub id: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(deserialize_with = "lenient_number")]
    pub width_mm: f64,
    #[serde(deserialize_with = "lenient_number")]
    pub height_mm: f64,
    /// A shape this version doesn't know reads as a rectangle.
    #[serde(deserialize_with = "lenient_shape")]
    pub shape: HoopShape,
    /// A cap frame: never chosen automatically.
    #[serde(skip_serializing_if = "std::ops::Not::not", deserialize_with = "lenient_bool")]
    pub cap: bool,
}

fn cap_chars(mut s: String) -> String {
    if let Some((i, _)) = s.char_indices().nth(MAX_HOOP_NAME) {
        s.truncate(i);
    }
    s
}

fn lenient_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(cap_chars(lenient(d)?.unwrap_or_default()))
}

fn lenient_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(lenient::<D, String>(d)?.map(cap_chars))
}

fn lenient_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(lenient(d)?.unwrap_or(0.0))
}

fn lenient_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(lenient(d)?.unwrap_or(false))
}

fn lenient_shape<'de, D: serde::Deserializer<'de>>(d: D) -> Result<HoopShape, D::Error> {
    Ok(lenient(d)?.unwrap_or_default())
}

impl Hoop {
    /// Whether the size is finite, positive and at most [`MAX_HOOP_MM`]: a damaged hoop is ignored
    /// rather than used.
    pub fn valid(&self) -> bool {
        let side = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_HOOP_MM;
        side(self.width_mm) && side(self.height_mm)
    }

    /// The sewable area in mm².
    pub fn area(&self) -> f64 {
        match self.shape {
            HoopShape::Rectangle => self.width_mm * self.height_mm,
            HoopShape::Round => std::f64::consts::FRAC_PI_4 * self.width_mm * self.height_mm,
        }
    }

    /// Whether the box (`x0`, `y0`)–(`x1`, `y1`), in mm from the hoop's centre, lies inside the
    /// hoop at least `margin` mm from its edge.
    pub fn holds(&self, (x0, y0, x1, y1): (f64, f64, f64, f64), margin: f64) -> bool {
        let (a, b) = (self.width_mm / 2.0 - margin, self.height_mm / 2.0 - margin);
        if !(a > 0.0 && b > 0.0) {
            return false;
        }
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
        match self.shape {
            HoopShape::Rectangle => corners.iter().all(|&(x, y)| x.abs() <= a + 1e-9 && y.abs() <= b + 1e-9),
            // Every corner inside the ellipse shrunk by the margin: the box is inside it then too.
            HoopShape::Round => corners.iter().all(|&(x, y)| (x / a).powi(2) + (y / b).powi(2) <= 1.0 + 1e-9),
        }
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
    pub fn hoop(&self) -> Option<&Hoop> {
        self.hoop.as_ref().filter(|h| h.valid())
    }

    /// The fabric colour: the one set, else [`DEFAULT_FABRIC`].
    pub fn fabric(&self) -> [u8; 3] {
        self.fabric.unwrap_or(DEFAULT_FABRIC)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn round_trips_and_reads_leniently() {
        let d = StitchDesign { order: vec![NodeId(3), NodeId(1)], start: Some(Point::new(1.0, 2.0)), end: None, hoop: None, fabric: None };
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
        let h = Hoop { id: Some("square-100".into()), name: "Square".into(), width_mm: 100.0, height_mm: 100.0, shape: HoopShape::Round, cap: false };
        let d = StitchDesign { hoop: Some(h.clone()), ..Default::default() };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["hoop"]["widthMm"], 100.0);
        assert_eq!(v["hoop"]["shape"], "round");
        assert!(v["hoop"].get("cap").is_none());
        assert_eq!(serde_json::from_value::<StitchDesign>(v).unwrap().hoop(), Some(&h));
        let custom = Hoop { id: None, ..h };
        assert!(serde_json::to_value(&custom).unwrap().get("id").is_none());
        let bad: StitchDesign =
            serde_json::from_value(serde_json::json!({"hoop": {"widthMm": "x", "heightMm": 1e9, "name": 5, "shape": "star"}})).unwrap();
        assert!(bad.hoop.is_some() && bad.hoop().is_none());
        assert_eq!(bad.hoop.map(|h| h.shape), Some(HoopShape::Rectangle));
        let junk: StitchDesign = serde_json::from_value(serde_json::json!({"hoop": 7})).unwrap();
        assert_eq!(junk.hoop, None);
        let long: StitchDesign =
            serde_json::from_value(serde_json::json!({"hoop": {"name": "n".repeat(500), "widthMm": 10, "heightMm": 10}})).unwrap();
        assert_eq!(long.hoop().unwrap().name.len(), MAX_HOOP_NAME);
    }

    #[test]
    fn the_fabric_round_trips_as_hex_and_damage_reads_as_the_default() {
        let d = StitchDesign { fabric: Some([10, 200, 255]), ..Default::default() };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["fabric"], "#0ac8ff");
        assert_eq!(serde_json::from_value::<StitchDesign>(v).unwrap().fabric(), [10, 200, 255]);
        assert!(serde_json::to_value(StitchDesign::default()).unwrap().get("fabric").is_none());
        assert_eq!(StitchDesign::default().fabric(), DEFAULT_FABRIC);
        for bad in [json!(7), json!("#12345"), json!("#gggggg"), json!([1, 2, 3]), json!("#ééé")] {
            let back: StitchDesign = serde_json::from_value(json!({"fabric": bad, "order": [2]})).unwrap();
            assert_eq!((back.fabric, back.order.len()), (None, 1), "{bad}");
        }
        assert_eq!(parse_rgb_hex("ECE8DE"), Some(DEFAULT_FABRIC));
        assert_eq!(rgb_hex(DEFAULT_FABRIC), "#ece8de");
    }

    #[test]
    fn hoops_hold_boxes_by_their_shape() {
        let r = Hoop { name: "r".into(), width_mm: 150.0, height_mm: 150.0, shape: HoopShape::Round, ..Default::default() };
        assert!(r.holds((-45.0, -45.0, 45.0, 45.0), 5.0));
        assert!(!r.holds((-50.0, -50.0, 50.0, 50.0), 5.0));
        let s = Hoop { width_mm: 100.0, height_mm: 100.0, shape: HoopShape::Rectangle, ..r };
        assert!(s.holds((-40.0, -40.0, 40.0, 40.0), 10.0));
        assert!(!s.holds((-30.0, -40.0, 50.0, 40.0), 10.0), "off centre");
        assert!(!s.holds((-0.5, -0.5, 0.5, 0.5), 60.0), "a margin past the middle holds nothing");
        assert!(!s.holds((f64::NAN, 0.0, 1.0, 1.0), 0.0));
        assert_eq!(HoopShape::parse("oval"), Some(HoopShape::Round));
        assert_eq!(HoopShape::parse("star"), None);
        assert_eq!(HoopShape::Rectangle.id(), "rectangle");
    }
}
