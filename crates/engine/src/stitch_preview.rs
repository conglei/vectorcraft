//! The stitch preview a window shows while designing: the plan an export would sew, summarised per
//! colour block, and pictures of it (all of it, or its first N stitches for playback). It is the
//! engine's `stitch.preview` without the PNG and the session, so a panel can build the plan on a
//! worker thread from a shared document and draw frames from the plan it keeps.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_stitch::{
    MARGIN_MM, MAX_SCALE, MIN_SCALE, PreviewMode, PreviewOptions, Region, Step, StitchPlan, UNITS_PER_MM, preview_prefix, preview_region,
    preview_size, stats,
};

use crate::cmd::fileio::stitchfile;

/// No side of a frame is longer than this many pixels.
pub const MAX_FRAME_SIDE: u32 = 4096;
/// The most pixels per millimetre a frame is drawn at.
pub const FRAME_SCALE: f64 = 10.0;

/// One thread block of the plan, in sewing order.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub rgb: [u8; 3],
    pub stitches: usize,
    pub length_mm: f64,
    /// The number of stitches sewn before this block starts.
    pub first: usize,
}

/// A machine function between stitches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Trim,
    Stop,
}

/// What the plan holds.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanSummary {
    pub stitches: usize,
    pub jumps: usize,
    pub trims: usize,
    pub color_changes: usize,
    pub width_mm: f64,
    pub height_mm: f64,
    pub blocks: Vec<BlockInfo>,
    /// The trims and stops in sewing order, each with the number of stitches sewn before it. A
    /// colour change is where a block starts ([`BlockInfo::first`]).
    pub marks: Vec<(usize, Mark)>,
    pub warnings: Vec<String>,
}

impl PlanSummary {
    /// The block the `n`-th stitch (1-based; 0 is before any) is in, if there is one.
    pub fn block_at(&self, n: usize) -> Option<usize> {
        if n == 0 {
            return None;
        }
        self.blocks.iter().rposition(|b| b.stitches > 0 && b.first < n)
    }
}

/// A plan built from a document, with its summary.
#[derive(Clone, Debug)]
pub struct BuiltPlan {
    plan: Arc<StitchPlan>,
    pub summary: PlanSummary,
}

/// How to draw a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameOptions {
    /// The density view instead of lit thread.
    pub density: bool,
    pub show_jumps: bool,
    pub show_needles: bool,
    /// Rings at trims, colour changes and stops.
    pub show_functions: bool,
    /// Only the first this many stitches (`None`: all of them).
    pub stitches: Option<usize>,
}

/// The fabric colour pictures are drawn on, and a canvas showing a [`BuiltPlan::region`] paints.
pub const FABRIC: [u8; 3] = vectorcraft_stitch::DEFAULT_FABRIC;
/// The most pixels a side of a region.
pub const MAX_REGION_SIDE: u32 = vectorcraft_stitch::MAX_SIDE;
/// The pixels per millimetre a region can be drawn at.
pub const REGION_SCALES: std::ops::RangeInclusive<f64> = vectorcraft_stitch::MIN_REGION_SCALE..=MAX_SCALE;
/// Millimetres per document point.
pub const MM_PER_PT: f64 = 1.0 / vectorcraft_stitch::PT_PER_MM;

/// A rectangle of the document to draw the plan's stitches in, for a canvas that shows them in
/// place: the plan's coordinates are the document's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionOptions {
    pub frame: FrameOptions,
    /// The picture's top-left corner, in millimetres of the document's space.
    pub left_mm: f64,
    pub top_mm: f64,
    /// Pixels per millimetre, within [`REGION_SCALES`].
    pub scale: f64,
    /// The picture's size in pixels, at most [`MAX_REGION_SIDE`] a side.
    pub width: u32,
    pub height: u32,
}

/// A picture of the plan: RGBA rows, straight alpha; opaque from [`BuiltPlan::frame`].
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub max_coverage: f64,
    pub dense_cells: usize,
}

/// The plan an export of `doc` would write, for the plan options in `options` (those of
/// `stitch.info`; the machine defaults to `generic`). Errors say why there is no plan: no stroked
/// paths, bad options, too many stitches.
pub fn build_plan(doc: &Document, options: &Value) -> Result<BuiltPlan, String> {
    let mut p = options.as_object().cloned().unwrap_or_default();
    // The selection is not what a preview shows, nor where a file would go.
    p.remove("selectedOnly");
    p.remove("path");
    p.entry("machine").or_insert_with(|| json!("generic"));
    p.retain(|_, v| !v.is_null());
    let p = Value::Object(p);
    let (plan, warnings) = stitchfile::plan("stitch.preview", doc, &p, "dst", None).map_err(|e| e.to_string())?;
    let st = stats(&plan);
    let mut first = 0;
    let mut marks = Vec::new();
    let blocks = plan
        .blocks
        .iter()
        .map(|b| {
            let (mut n, mut units, mut prev) = (0usize, 0.0_f64, None);
            for s in &b.steps {
                match s {
                    Step::Stitch(p) => {
                        n += 1;
                        if let Some(q) = prev {
                            units += p.dist(q);
                        }
                        prev = Some(*p);
                    }
                    Step::Jump(_) => prev = None,
                    Step::Trim => {
                        marks.push((first + n, Mark::Trim));
                        prev = None;
                    }
                    Step::Stop => {
                        marks.push((first + n, Mark::Stop));
                        prev = None;
                    }
                }
            }
            let info = BlockInfo { rgb: b.thread.rgb, stitches: n, length_mm: units / UNITS_PER_MM, first };
            first += n;
            info
        })
        .collect();
    let summary = PlanSummary {
        stitches: st.stitches,
        jumps: st.jumps,
        trims: st.trims,
        color_changes: st.color_changes,
        width_mm: st.width_mm,
        height_mm: st.height_mm,
        blocks,
        marks,
        warnings,
    };
    Ok(BuiltPlan { plan: Arc::new(plan), summary })
}

impl BuiltPlan {
    /// The plan drawn with `o`, at the largest scale that keeps it within [`MAX_FRAME_SIDE`]
    /// pixels a side (at most [`FRAME_SCALE`] pixels per mm). The same size for every `stitches`.
    pub fn frame(&self, o: &FrameOptions) -> Result<Frame, String> {
        let mut view = PreviewOptions {
            show_jumps: o.show_jumps,
            show_needles: o.show_needles,
            show_functions: o.show_functions,
            mode: if o.density { PreviewMode::Density } else { PreviewMode::Realistic },
            ..PreviewOptions::default()
        };
        // Start from the design's size, then correct once for what the layout adds (jumps widen it).
        let span = self.summary.width_mm.max(self.summary.height_mm) + 2.0 * MARGIN_MM;
        view.scale = (f64::from(MAX_FRAME_SIDE) / span.max(1.0)).clamp(MIN_SCALE, FRAME_SCALE.min(MAX_SCALE));
        let (w, h) = preview_size(&self.plan, &view)?;
        let side = w.max(h);
        if side > MAX_FRAME_SIDE {
            view.scale = (view.scale * f64::from(MAX_FRAME_SIDE) / f64::from(side) * 0.999).max(MIN_SCALE);
        }
        let img = preview_prefix(&self.plan, &view, o.stitches.unwrap_or(usize::MAX))?;
        if img.width.max(img.height) > MAX_FRAME_SIDE {
            return Err(format!("the preview would be {} × {} pixels, more than {MAX_FRAME_SIDE} a side", img.width, img.height));
        }
        Ok(Frame { width: img.width, height: img.height, rgba: img.rgba, max_coverage: img.max_coverage, dense_cells: img.dense_cells })
    }
}

impl BuiltPlan {
    /// Where the needle is after the `n`-th stitch (1-based), in millimetres of the document's
    /// space. `None` for 0 and past the last stitch.
    pub fn needle_mm(&self, n: usize) -> Option<(f64, f64)> {
        let at = n.checked_sub(1)?;
        self.plan
            .blocks
            .iter()
            .flat_map(|b| b.steps.iter())
            .filter_map(|s| match s {
                Step::Stitch(p) => Some(*p),
                _ => None,
            })
            .nth(at)
            .map(vectorcraft_stitch::Pt::to_mm)
    }

    /// The stitches inside `r`, drawn as [`BuiltPlan::frame`] draws them but with no fabric: where
    /// there is no thread or mark the picture is transparent, to lay over the document's page.
    /// Errors for a scale outside [`REGION_SCALES`], a side of 0 or more than
    /// [`MAX_REGION_SIDE`] pixels, or a corner that is not finite.
    pub fn region(&self, r: &RegionOptions) -> Result<Frame, String> {
        let view = PreviewOptions {
            scale: r.scale,
            show_jumps: r.frame.show_jumps,
            show_needles: r.frame.show_needles,
            show_functions: r.frame.show_functions,
            mode: if r.frame.density { PreviewMode::Density } else { PreviewMode::Realistic },
            ..PreviewOptions::default()
        };
        let region = Region { left_mm: r.left_mm, top_mm: r.top_mm, width: r.width, height: r.height, transparent: true };
        let img = preview_region(&self.plan, &view, &region, r.frame.stitches)?;
        Ok(Frame { width: img.width, height: img.height, rgba: img.rgba, max_coverage: img.max_coverage, dense_cells: img.dense_cells })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        s.execute("paint.setFill", &json!({"none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
        s.execute("shape.ellipse", &json!({"x": 20, "y": 80, "width": 60, "height": 40})).unwrap();
        s
    }

    #[test]
    fn the_plan_is_summarised_per_block_and_drawn_in_frames_of_one_size() {
        let s = session();
        let doc = s.active().unwrap().doc.clone();
        let built = build_plan(&doc, &json!({})).unwrap();
        let sum = &built.summary;
        let info = stats(&built.plan);
        assert_eq!(sum.stitches, info.stitches);
        assert!(sum.blocks.len() >= 2 && sum.stitches > 20, "{sum:?}");
        assert_eq!(sum.blocks.iter().map(|b| b.stitches).sum::<usize>(), sum.stitches);
        assert_eq!(sum.blocks[1].first, sum.blocks[0].stitches);
        assert!(sum.blocks.iter().all(|b| b.length_mm > 0.0));
        assert_eq!(sum.block_at(0), None);
        assert_eq!(sum.block_at(1), Some(0));
        assert_eq!(sum.block_at(sum.blocks[0].stitches + 1), Some(1));
        let whole = built.frame(&FrameOptions::default()).unwrap();
        let part = built.frame(&FrameOptions { stitches: Some(5), show_needles: true, ..FrameOptions::default() }).unwrap();
        assert_eq!((whole.width, whole.height), (part.width, part.height));
        assert_eq!(whole.rgba.len(), whole.width as usize * whole.height as usize * 4);
        assert_ne!(whole.rgba, part.rgba);
        let dense = built.frame(&FrameOptions { density: true, ..FrameOptions::default() }).unwrap();
        assert!(dense.max_coverage > 0.0);
    }

    #[test]
    fn frames_stay_within_the_side_cap_and_an_empty_document_is_an_error() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        assert!(build_plan(&doc, &json!({})).is_err());
        // A path about 2 m across: the scale drops to fit.
        s.execute("paint.setFill", &json!({"none": true})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#00aa00"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5000, "height": 100})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        let f = build_plan(&doc, &json!({})).unwrap().frame(&FrameOptions::default()).unwrap();
        assert!(f.width <= MAX_FRAME_SIDE && f.height <= MAX_FRAME_SIDE, "{} × {}", f.width, f.height);
        // Bad options are errors, not panics.
        assert!(build_plan(&doc, &json!({"stitchLength": -1})).is_err());
    }

    #[test]
    fn a_region_shows_the_stitches_where_the_document_has_them() {
        let s = session();
        let doc = s.active().unwrap().doc.clone();
        let built = build_plan(&doc, &json!({})).unwrap();
        // The red rectangle spans 0..100 pt × 0..50 pt: its top edge is the line y = 0 mm.
        let r = RegionOptions { frame: FrameOptions::default(), left_mm: -5.0, top_mm: -5.0, scale: 4.0, width: 200, height: 120 };
        let f = built.region(&r).unwrap();
        assert_eq!((f.width, f.height, f.rgba.len()), (200, 120, 200 * 120 * 4));
        let at = |x: usize, y: usize| &f.rgba[(y * 200 + x) * 4..][..4];
        // (10 mm, 0 mm) is pixel (60, 20): red thread. (10 mm, 5 mm), inside the rectangle: nothing.
        // The thread is 1.6 pixels wide at this scale, so it covers most of the pixel, not all.
        assert!(at(60, 20)[3] > 150, "{:?}", at(60, 20));
        assert!(at(60, 20)[0] > at(60, 20)[2] + 60, "{:?}", at(60, 20));
        assert_eq!(at(60, 40)[3], 0);
        // The same place whatever the playback position; before any stitch, nothing.
        let none = built.region(&RegionOptions { frame: FrameOptions { stitches: Some(0), ..FrameOptions::default() }, ..r }).unwrap();
        assert!(none.rgba.chunks(4).all(|p| p[3] == 0));
        // Somewhere else in the document: nothing, and no error.
        let far = built.region(&RegionOptions { left_mm: 5000.0, ..r }).unwrap();
        assert!(far.rgba.chunks(4).all(|p| p[3] == 0));
        // Bad regions are errors, not panics or huge allocations.
        for bad in [
            RegionOptions { scale: 0.0, ..r },
            RegionOptions { scale: f64::NAN, ..r },
            RegionOptions { scale: 1e9, ..r },
            RegionOptions { width: 0, ..r },
            RegionOptions { height: u32::MAX, ..r },
            RegionOptions { left_mm: f64::INFINITY, ..r },
        ] {
            assert!(built.region(&bad).is_err(), "{bad:?}");
        }
        // The needle after the first stitch is on the rectangle's outline; none before or past the end.
        let (x, y) = built.needle_mm(1).unwrap();
        assert!((-0.1..=35.4).contains(&x) && (-0.1..=17.8).contains(&y), "({x}, {y})");
        assert_eq!(built.needle_mm(0), None);
        assert_eq!(built.needle_mm(built.summary.stitches + 1), None);
        assert!(built.needle_mm(built.summary.stitches).is_some());
        // The trims lie between the first stitch and the last, in order.
        let marks = &built.summary.marks;
        assert_eq!(marks.iter().filter(|m| m.1 == Mark::Trim).count(), built.summary.trims, "{marks:?}");
        assert!(marks.windows(2).all(|w| w[0].0 <= w[1].0) && marks.iter().all(|m| m.0 <= built.summary.stitches), "{marks:?}");
        assert!(REGION_SCALES.contains(&4.0) && MAX_REGION_SIDE >= MAX_FRAME_SIDE);
        assert!((MM_PER_PT - 25.4 / 72.0).abs() < 1e-12);
    }
}
