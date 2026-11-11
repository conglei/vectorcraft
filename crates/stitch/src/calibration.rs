//! The calibration design: a 100 mm test piece for a first stitch-out, drawn in code.
//!
//! It has a square to check size and registration, ticks along the top edge to read the scale,
//! an F to check direction and mirroring, and a near and a far pair of squares to check
//! registration between colours at two distances. The first stitch is the square's corner at the
//! top-left, which is off-centre on purpose.

use vectorcraft_geom::{PathData, Point, SubPath};

use crate::PT_PER_MM;

const BLACK: [u8; 3] = [0, 0, 0];
const RED: [u8; 3] = [200, 30, 40];
const BLUE: [u8; 3] = [30, 90, 200];

/// One part of the calibration design, stitched as one colour in the order of the list.
pub struct CalibrationPart {
    /// What the part is for.
    pub name: &'static str,
    /// The part's paths in document points from the design's top-left corner, y down, every anchor
    /// a corner.
    pub path: PathData,
    /// The thread colour.
    pub rgb: [u8; 3],
}

fn polyline(mm: &[(f64, f64)], closed: bool) -> SubPath {
    let points: Vec<Point> = mm.iter().map(|&(x, y)| Point::new(x * PT_PER_MM, y * PT_PER_MM)).collect();
    SubPath::polyline(&points, closed)
}

fn square(x: f64, y: f64, side: f64) -> SubPath {
    polyline(&[(x, y), (x + side, y), (x + side, y + side), (x, y + side)], true)
}

/// The calibration design in stitching order: `square`, `ticks`, `F`, `near pair`, `far pair`.
pub fn calibration() -> Vec<CalibrationPart> {
    let ticks: Vec<SubPath> = (0..=10)
        .map(|i| {
            let x = f64::from(i) * 10.0;
            let long = i == 0 || i == 5 || i == 10;
            polyline(&[(x, 0.0), (x, if long { 6.0 } else { 3.0 })], false)
        })
        .collect();
    vec![
        CalibrationPart { name: "square", path: PathData::single(square(0.0, 0.0, 100.0)), rgb: BLACK },
        CalibrationPart { name: "ticks", path: PathData::new(ticks), rgb: BLACK },
        CalibrationPart {
            name: "F",
            path: PathData::single(polyline(&[(5.0, 35.0), (5.0, 10.0), (20.0, 10.0), (5.0, 10.0), (5.0, 22.0), (15.0, 22.0)], false)),
            rgb: RED,
        },
        CalibrationPart { name: "near pair", path: PathData::new(vec![square(30.0, 40.0, 10.0), square(45.0, 40.0, 10.0)]), rgb: BLUE },
        CalibrationPart { name: "far pair", path: PathData::new(vec![square(30.0, 70.0, 10.0), square(70.0, 70.0, 10.0)]), rgb: BLUE },
    ]
}

#[cfg(test)]
mod tests {
    use vectorcraft_geom::Rect;

    use super::*;

    #[test]
    fn the_design_is_100_mm_square_in_three_colours() {
        let parts = calibration();
        let mut colours: Vec<[u8; 3]> = parts.iter().map(|p| p.rgb).collect();
        colours.dedup();
        assert_eq!(colours.len(), 3);
        let b = parts.iter().filter_map(|p| p.path.bounds()).fold(None::<Rect>, |a, r| Some(a.map_or(r, |a| a.union(r)))).unwrap();
        let side = 100.0 * PT_PER_MM;
        assert!((b.x0).abs() < 1e-6 && (b.y0).abs() < 1e-6, "{b:?}");
        assert!((b.width() - side).abs() < 1e-6, "{b:?}");
        assert!((b.height() - side).abs() < 1e-6, "{b:?}");
        assert_eq!(parts.iter().map(|p| p.name).collect::<Vec<_>>(), ["square", "ticks", "F", "near pair", "far pair"]);
    }
}
