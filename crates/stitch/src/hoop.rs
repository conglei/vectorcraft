//! Hoops and frames: the area a machine can sew in one hooping.
//!
//! The built-in library is generic sizes by their inner dimensions, chosen for VectorCraft from
//! the sizes commonly sold for commercial multi-needle machines; it names no maker and copies no
//! catalogue. A hoop's size is its sewable inner area, width along the machine's x axis.

use crate::MachineProfile;

/// The largest hoop side accepted, in mm (bigger than any frame a single-head machine takes).
pub const MAX_HOOP_MM: f64 = 2000.0;
/// The smallest hoop side accepted, in mm.
pub const MIN_HOOP_MM: f64 = 10.0;
/// The default clearance between the design and the hoop's edge, in mm.
pub const DEFAULT_MARGIN_MM: f64 = 10.0;
/// The largest margin accepted, in mm.
pub const MAX_MARGIN_MM: f64 = 100.0;

/// A hoop's outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoopShape {
    /// A rectangle (or a rounded rectangle: the sewable area is taken as the full rectangle).
    Rect,
    /// A circle or an oval whose sewable area is the ellipse inside width × height.
    Round,
}

impl HoopShape {
    /// The id used in files and commands.
    pub fn id(self) -> &'static str {
        match self {
            HoopShape::Rect => "rect",
            HoopShape::Round => "round",
        }
    }

    /// The shape an id names (`rect`, `round`).
    pub fn parse(s: &str) -> Option<HoopShape> {
        match s {
            "rect" | "rectangle" => Some(HoopShape::Rect),
            "round" | "oval" | "circle" => Some(HoopShape::Round),
            _ => None,
        }
    }
}

/// One hoop or frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Hoop {
    pub id: String,
    pub name: String,
    /// Inner width in mm.
    pub width_mm: f64,
    /// Inner height in mm.
    pub height_mm: f64,
    pub shape: HoopShape,
    /// A cap frame: never chosen automatically.
    pub cap: bool,
}

/// `(id, name, width, height, shape, cap)` of the built-in hoops, smallest first.
const LIBRARY: &[(&str, &str, f64, f64, HoopShape, bool)] = &[
    ("square-100", "Square 100 × 100 mm", 100.0, 100.0, HoopShape::Rect, false),
    ("round-150", "Round 150 mm", 150.0, 150.0, HoopShape::Round, false),
    ("rect-130x180", "Rectangle 130 × 180 mm", 130.0, 180.0, HoopShape::Rect, false),
    ("square-200", "Square 200 × 200 mm", 200.0, 200.0, HoopShape::Rect, false),
    ("round-240", "Round 240 mm", 240.0, 240.0, HoopShape::Round, false),
    ("rect-300x200", "Rectangle 300 × 200 mm", 300.0, 200.0, HoopShape::Rect, false),
    ("rect-360x200", "Rectangle 360 × 200 mm", 360.0, 200.0, HoopShape::Rect, false),
    ("rect-400x300", "Rectangle 400 × 300 mm", 400.0, 300.0, HoopShape::Rect, false),
    ("frame-500x350", "Tubular frame 500 × 350 mm", 500.0, 350.0, HoopShape::Rect, false),
    ("cap-360x80", "Cap frame 360 × 80 mm", 360.0, 80.0, HoopShape::Rect, true),
];

/// The built-in hoops, smallest first.
pub fn hoop_library() -> Vec<Hoop> {
    LIBRARY.iter().map(|&(id, name, w, h, shape, cap)| Hoop { id: id.into(), name: name.into(), width_mm: w, height_mm: h, shape, cap }).collect()
}

/// The built-in hoop with this id.
pub fn library_hoop(id: &str) -> Option<Hoop> {
    hoop_library().into_iter().find(|h| h.id == id)
}

/// The built-in hoops that fit the machine's field (all of them when it has none).
pub fn hoops_for(profile: &MachineProfile) -> Vec<Hoop> {
    hoop_library().into_iter().filter(|h| h.within_field(profile)).collect()
}

impl Hoop {
    /// A hoop of this size, checked: sides finite and within [`MIN_HOOP_MM`]–[`MAX_HOOP_MM`].
    pub fn custom(name: &str, width_mm: f64, height_mm: f64, shape: HoopShape) -> Result<Hoop, String> {
        for (k, v) in [("width", width_mm), ("height", height_mm)] {
            if !(v.is_finite() && (MIN_HOOP_MM..=MAX_HOOP_MM).contains(&v)) {
                return Err(format!("the hoop's {k} must be {MIN_HOOP_MM}–{MAX_HOOP_MM} mm, not {v}"));
            }
        }
        let name = if name.trim().is_empty() { format!("Custom {} × {} mm", trim(width_mm), trim(height_mm)) } else { name.trim().to_string() };
        Ok(Hoop { id: "custom".into(), name, width_mm, height_mm, shape, cap: false })
    }

    /// The sewable area in mm².
    pub fn area(&self) -> f64 {
        match self.shape {
            HoopShape::Rect => self.width_mm * self.height_mm,
            HoopShape::Round => std::f64::consts::FRAC_PI_4 * self.width_mm * self.height_mm,
        }
    }

    /// Whether the hoop fits the machine's field.
    pub fn within_field(&self, profile: &MachineProfile) -> bool {
        profile.field_mm.is_none_or(|(fw, fh)| self.width_mm <= fw + 1e-9 && self.height_mm <= fh + 1e-9)
    }

    /// Whether the box (`x0`, `y0`)–(`x1`, `y1`), in mm from the hoop's centre, lies inside the hoop
    /// at least `margin` mm from its edge.
    pub fn holds(&self, (x0, y0, x1, y1): (f64, f64, f64, f64), margin: f64) -> bool {
        let (a, b) = (self.width_mm / 2.0 - margin, self.height_mm / 2.0 - margin);
        if !(a > 0.0 && b > 0.0) {
            return false;
        }
        let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
        match self.shape {
            HoopShape::Rect => corners.iter().all(|&(x, y)| x.abs() <= a + 1e-9 && y.abs() <= b + 1e-9),
            // Every corner inside the ellipse shrunk by the margin: the box is inside it then too.
            HoopShape::Round => corners.iter().all(|&(x, y)| (x / a).powi(2) + (y / b).powi(2) <= 1.0 + 1e-9),
        }
    }

    /// Whether a design `w` × `h` mm fits, centred, with `margin` mm to spare.
    pub fn fits(&self, w: f64, h: f64, margin: f64) -> bool {
        self.holds((-w / 2.0, -h / 2.0, w / 2.0, h / 2.0), margin)
    }
}

/// The smallest hoop of `hoops` (cap frames left out) that a `w` × `h` mm design fits with `margin`.
pub fn smallest_fit(hoops: &[Hoop], w: f64, h: f64, margin: f64) -> Option<Hoop> {
    hoops.iter().filter(|o| !o.cap && o.fits(w, h, margin)).min_by(|a, b| a.area().total_cmp(&b.area())).cloned()
}

/// `v` without a trailing `.0`.
fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BAI_MIRROR, GENERIC, MachineProfile};

    #[test]
    fn the_library_is_ordered_and_fits_the_commercial_field() {
        let lib = hoop_library();
        assert_eq!(lib.len(), 10);
        assert_eq!(hoops_for(&BAI_MIRROR).len(), 10);
        assert_eq!(hoops_for(&GENERIC).len(), 10);
        let small = MachineProfile { field_mm: Some((200.0, 200.0)), ..GENERIC };
        let ids: Vec<String> = hoops_for(&small).into_iter().map(|h| h.id).collect();
        assert_eq!(ids, ["square-100", "round-150", "rect-130x180", "square-200"]);
        assert!(library_hoop("cap-360x80").unwrap().cap);
        assert_eq!(library_hoop("nope"), None);
    }

    #[test]
    fn fit_takes_the_smallest_hoop_with_the_margin() {
        let lib = hoop_library();
        // 100 mm square with 10 mm each side: 120, so not the 100 square; the 150 circle needs a
        // half-diagonal of 70.7 within 65: no; 130 × 180 holds 120 × 120.
        assert_eq!(smallest_fit(&lib, 100.0, 100.0, 10.0).unwrap().id, "rect-130x180");
        assert_eq!(smallest_fit(&lib, 80.0, 80.0, 10.0).unwrap().id, "square-100");
        assert_eq!(smallest_fit(&lib, 80.0, 80.0, 0.0).unwrap().id, "square-100");
        assert_eq!(smallest_fit(&lib, 480.0, 330.0, 10.0).unwrap().id, "frame-500x350");
        assert_eq!(smallest_fit(&lib, 481.0, 330.0, 10.0), None);
        // A long thin design would fit the cap frame, which is never chosen automatically.
        assert_eq!(smallest_fit(&lib, 300.0, 50.0, 10.0).unwrap().id, "rect-360x200");
    }

    #[test]
    fn round_hoops_hold_the_corners_and_rect_hoops_the_box() {
        let r = library_hoop("round-150").unwrap();
        assert!(r.fits(90.0, 90.0, 5.0));
        assert!(!r.fits(100.0, 100.0, 5.0));
        let s = library_hoop("square-100").unwrap();
        assert!(s.holds((-40.0, -40.0, 40.0, 40.0), 10.0));
        assert!(!s.holds((-30.0, -40.0, 50.0, 40.0), 10.0), "off centre");
        assert!(!s.fits(1.0, 1.0, 60.0), "a margin past the middle holds nothing");
        assert!(!s.fits(f64::NAN, 1.0, 0.0));
    }

    #[test]
    fn custom_hoops_are_checked() {
        let h = Hoop::custom("", 180.0, 130.5, HoopShape::Round).unwrap();
        assert_eq!(h.name, "Custom 180 × 130.5 mm");
        assert_eq!(h.id, "custom");
        for (w, hh) in [(5.0, 100.0), (100.0, 3000.0), (f64::NAN, 100.0), (f64::INFINITY, 100.0)] {
            assert!(Hoop::custom("x", w, hh, HoopShape::Rect).is_err());
        }
        assert_eq!(HoopShape::parse("oval"), Some(HoopShape::Round));
        assert_eq!(HoopShape::parse("star"), None);
        assert_eq!(HoopShape::Rect.id(), "rect");
    }
}
