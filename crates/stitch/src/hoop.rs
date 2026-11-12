//! Hoops and frames: the area a machine can sew in one hooping ([`Hoop`], kept in the document).
//!
//! The built-in library is generic sizes by their inner dimensions, chosen for VectorCraft from
//! the sizes commonly sold for commercial multi-needle machines; it names no maker and copies no
//! catalogue. A hoop's size is its sewable inner area, width along the machine's x axis.

pub use vectorcraft_doc::{Hoop, HoopShape};

/// The largest hoop side accepted, in mm (bigger than any frame a single-head machine takes).
pub const MAX_HOOP_MM: f64 = vectorcraft_doc::stitch_design::MAX_HOOP_MM;
/// The smallest hoop side accepted, in mm.
pub const MIN_HOOP_MM: f64 = 10.0;
/// The default clearance between the design and the hoop's edge, in mm.
pub const DEFAULT_MARGIN_MM: f64 = 10.0;
/// The largest margin accepted, in mm.
pub const MAX_MARGIN_MM: f64 = 100.0;

/// `(id, name, width, height, shape, cap)` of the built-in hoops, smallest first.
const LIBRARY: &[(&str, &str, f64, f64, HoopShape, bool)] = &[
    ("square-100", "Square 100 × 100 mm", 100.0, 100.0, HoopShape::Rectangle, false),
    ("round-150", "Round 150 mm", 150.0, 150.0, HoopShape::Round, false),
    ("rect-130x180", "Rectangle 130 × 180 mm", 130.0, 180.0, HoopShape::Rectangle, false),
    ("square-200", "Square 200 × 200 mm", 200.0, 200.0, HoopShape::Rectangle, false),
    ("round-240", "Round 240 mm", 240.0, 240.0, HoopShape::Round, false),
    ("rect-300x200", "Rectangle 300 × 200 mm", 300.0, 200.0, HoopShape::Rectangle, false),
    ("rect-360x200", "Rectangle 360 × 200 mm", 360.0, 200.0, HoopShape::Rectangle, false),
    ("rect-400x300", "Rectangle 400 × 300 mm", 400.0, 300.0, HoopShape::Rectangle, false),
    ("frame-500x350", "Tubular frame 500 × 350 mm", 500.0, 350.0, HoopShape::Rectangle, false),
    ("cap-360x80", "Cap frame 360 × 80 mm", 360.0, 80.0, HoopShape::Rectangle, true),
];

/// The built-in hoops, smallest first.
pub fn library() -> Vec<Hoop> {
    LIBRARY
        .iter()
        .map(|&(id, name, w, h, shape, cap)| Hoop { id: Some(id.into()), name: name.into(), width_mm: w, height_mm: h, shape, cap })
        .collect()
}

/// The built-in hoop with this id.
pub fn find(id: &str) -> Option<Hoop> {
    library().into_iter().find(|h| h.id.as_deref() == Some(id))
}

/// Whether `hoop` fits a machine field (any hoop when there is none).
pub fn within_field(hoop: &Hoop, field_mm: Option<(f64, f64)>) -> bool {
    field_mm.is_none_or(|(fw, fh)| hoop.width_mm <= fw + 1e-9 && hoop.height_mm <= fh + 1e-9)
}

/// The built-in hoops that fit the machine field, smallest first.
pub fn for_field(field_mm: Option<(f64, f64)>) -> Vec<Hoop> {
    library().into_iter().filter(|h| within_field(h, field_mm)).collect()
}

/// Whether a design `design_mm` (width, height) fits `hoop`, centred, `margin_mm` from its edge.
pub fn fits(design_mm: (f64, f64), hoop: &Hoop, margin_mm: f64) -> bool {
    let (w, h) = design_mm;
    hoop.holds((-w / 2.0, -h / 2.0, w / 2.0, h / 2.0), margin_mm)
}

/// The smallest built-in hoop by area (cap frames left out) that lies within the machine field and
/// that the design fits with the margin.
pub fn fit(design_mm: (f64, f64), margin_mm: f64, field_mm: Option<(f64, f64)>) -> Option<Hoop> {
    for_field(field_mm).into_iter().filter(|o| !o.cap && fits(design_mm, o, margin_mm)).min_by(|a, b| a.area().total_cmp(&b.area()))
}

/// A custom hoop of this size, checked: sides finite and within [`MIN_HOOP_MM`]–[`MAX_HOOP_MM`].
pub fn custom(name: &str, width_mm: f64, height_mm: f64, shape: HoopShape) -> Result<Hoop, String> {
    for (k, v) in [("width", width_mm), ("height", height_mm)] {
        if !(v.is_finite() && (MIN_HOOP_MM..=MAX_HOOP_MM).contains(&v)) {
            return Err(format!("the hoop's {k} must be {MIN_HOOP_MM}–{MAX_HOOP_MM} mm, not {v}"));
        }
    }
    let name = if name.trim().is_empty() { format!("Custom {} × {} mm", trim(width_mm), trim(height_mm)) } else { name.trim().to_string() };
    Ok(Hoop { id: None, name, width_mm, height_mm, shape, cap: false })
}

/// `v` without a trailing `.0`.
fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BAI_MIRROR, GENERIC};

    #[test]
    fn the_library_is_ordered_and_filtered_by_the_field() {
        assert_eq!(library().len(), 10);
        assert_eq!(for_field(BAI_MIRROR.field_mm).len(), 10);
        assert_eq!(for_field(GENERIC.field_mm).len(), 10);
        let ids: Vec<String> = for_field(Some((200.0, 200.0))).into_iter().filter_map(|h| h.id).collect();
        assert_eq!(ids, ["square-100", "round-150", "rect-130x180", "square-200"]);
        assert!(find("cap-360x80").unwrap().cap);
        assert_eq!(find("nope"), None);
    }

    #[test]
    fn fit_takes_the_smallest_hoop_with_the_margin() {
        let id = |d, m, f| fit(d, m, f).and_then(|h: Hoop| h.id);
        // 100 mm square with 10 mm each side: 120, so not the 100 square; the 150 circle needs a
        // half-diagonal of 70.7 within 65: no; 130 × 180 holds 120 × 120.
        assert_eq!(id((100.0, 100.0), 10.0, None).as_deref(), Some("rect-130x180"));
        assert_eq!(id((80.0, 80.0), 10.0, None).as_deref(), Some("square-100"));
        assert_eq!(id((480.0, 330.0), 10.0, None).as_deref(), Some("frame-500x350"));
        assert_eq!(id((481.0, 330.0), 10.0, None), None);
        // Within a smaller field the large frames are not offered.
        assert_eq!(id((300.0, 150.0), 10.0, Some((300.0, 300.0))), None);
        // A long thin design would fit the cap frame, which is never chosen automatically.
        assert_eq!(id((300.0, 50.0), 10.0, None).as_deref(), Some("rect-360x200"));
        assert!(fits((90.0, 90.0), &find("round-150").unwrap(), 5.0));
        assert!(!fits((f64::NAN, 1.0), &find("square-100").unwrap(), 0.0));
    }

    #[test]
    fn custom_hoops_are_checked() {
        let h = custom("", 180.0, 130.5, HoopShape::Round).unwrap();
        assert_eq!(h.name, "Custom 180 × 130.5 mm");
        assert_eq!(h.id, None);
        for (w, hh) in [(5.0, 100.0), (100.0, 3000.0), (f64::NAN, 100.0), (f64::INFINITY, 100.0)] {
            assert!(custom("x", w, hh, HoopShape::Rectangle).is_err());
        }
    }
}
