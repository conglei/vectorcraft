//! The design checker's plan rules: short and long stitches, needle-point density, trims and
//! thread colours against the machine. The hoop fit is checked by the caller, which knows the hoop
//! ([`crate::Hoop`]).

use crate::{MachineProfile, Pt, Step, StitchPlan, UNITS_PER_MM, stats};

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    pub fn id(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
}

/// One thing the checker found.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub level: Level,
    /// A stable id: `empty`, `shortStitches`, `longStitches`, `overMaxStitch`, `density`, `trims`,
    /// `colors`, and the caller's hoop codes.
    pub code: &'static str,
    pub message: String,
    pub count: Option<usize>,
    /// Where, in plan units (0.1 mm, document position): the first case, or the worst cell's centre.
    pub at: Option<Pt>,
}

/// The checker's thresholds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckLimits {
    /// Stitches shorter than this (mm) are reported: they tend to break thread and pile up.
    pub short_mm: f64,
    /// Stitches longer than this (mm) are reported: long loose stitches snag.
    pub long_mm: f64,
    /// The side of a density cell, mm.
    pub cell_mm: f64,
    /// Needle points per mm² in one cell above which it is reported as too dense.
    pub max_density: f64,
    /// Trims above this are reported.
    pub max_trims: usize,
}

/// Bounds on the thresholds: (short, long, cell, density, trims).
pub const SHORT_RANGE: (f64, f64) = (0.0, 5.0);
pub const LONG_RANGE: (f64, f64) = (1.0, 12.7);
pub const CELL_RANGE: (f64, f64) = (0.5, 10.0);
pub const DENSITY_RANGE: (f64, f64) = (0.1, 100.0);
pub const MAX_TRIMS_LIMIT: usize = 100_000;
/// The most density cells along a side; larger designs get larger cells.
const MAX_CELLS_SIDE: usize = 1024;

impl Default for CheckLimits {
    fn default() -> Self {
        CheckLimits { short_mm: 0.5, long_mm: 7.0, cell_mm: 2.0, max_density: 4.0, max_trims: 50 }
    }
}

/// The plan's findings, most serious first (ties in rule order).
pub fn check_plan(plan: &StitchPlan, profile: &MachineProfile, l: &CheckLimits) -> Vec<Finding> {
    let st = stats(plan);
    let mut out = Vec::new();
    if st.stitches == 0 {
        out.push(Finding { level: Level::Error, code: "empty", message: "the design has no stitches".into(), count: None, at: None });
        return out;
    }
    let (short, long) = (l.short_mm * UNITS_PER_MM, l.long_mm * UNITS_PER_MM);
    let (mut shorts, mut longs, mut over) = (0usize, 0usize, 0usize);
    let (mut first_short, mut first_long, mut first_over) = (None, None, None);
    for_each_stitch(plan, |a, b| {
        let d = a.dist(b);
        if d < short {
            shorts += 1;
            first_short.get_or_insert(b);
        }
        if d > long {
            longs += 1;
            first_long.get_or_insert(b);
        }
        if (a.x - b.x).abs() > profile.max_stitch || (a.y - b.y).abs() > profile.max_stitch {
            over += 1;
            first_over.get_or_insert(b);
        }
    });
    if over > 0 {
        let max = f64::from(profile.max_stitch) / UNITS_PER_MM;
        out.push(Finding {
            level: Level::Error,
            code: "overMaxStitch",
            message: format!("{over} stitches are longer than the machine's {max:.1} mm"),
            count: Some(over),
            at: first_over,
        });
    }
    if shorts > 0 {
        out.push(Finding {
            level: Level::Warning,
            code: "shortStitches",
            message: format!("{shorts} stitches are shorter than {:.1} mm", l.short_mm),
            count: Some(shorts),
            at: first_short,
        });
    }
    if longs > 0 {
        out.push(Finding {
            level: Level::Warning,
            code: "longStitches",
            message: format!("{longs} stitches are longer than {:.1} mm and may snag", l.long_mm),
            count: Some(longs),
            at: first_long,
        });
    }
    if let Some((cells, worst, at)) = dense_cells(plan, l.cell_mm, l.max_density) {
        out.push(Finding {
            level: Level::Warning,
            code: "density",
            message: format!(
                "{cells} areas of {c} × {c} mm have more than {:.1} needle points per mm² (up to {worst:.1})",
                l.max_density,
                c = l.cell_mm
            ),
            count: Some(cells),
            at: Some(at),
        });
    }
    if st.trims > l.max_trims {
        out.push(Finding {
            level: Level::Warning,
            code: "trims",
            message: format!("{} trims, more than {}: joining objects would save machine time", st.trims, l.max_trims),
            count: Some(st.trims),
            at: None,
        });
    }
    let colours = st.threads.len();
    match profile.needles {
        Some(n) if colours > usize::from(n) => out.push(Finding {
            level: Level::Warning,
            code: "colors",
            message: format!("{colours} thread colours and {n} needles: {} need a thread swap during the job", colours - usize::from(n)),
            count: Some(colours),
            at: None,
        }),
        _ => out.push(Finding {
            level: Level::Info,
            code: "colors",
            message: format!("{colours} thread colours, {} colour changes", st.color_changes),
            count: Some(st.color_changes),
            at: None,
        }),
    }
    out.sort_by(|a, b| b.level.cmp(&a.level));
    out
}

fn for_each_stitch(plan: &StitchPlan, mut f: impl FnMut(Pt, Pt)) {
    for b in &plan.blocks {
        let mut prev: Option<Pt> = None;
        for s in &b.steps {
            match s {
                Step::Stitch(p) => {
                    if let Some(q) = prev {
                        f(q, *p);
                    }
                    prev = Some(*p);
                }
                Step::Jump(_) | Step::Trim | Step::Stop => prev = None,
            }
        }
    }
}

/// Needle points per cell over a grid of `cell_mm` cells (larger when the design would need more
/// than [`MAX_CELLS_SIDE`] along a side): the cells above `max` per mm², the highest density, and
/// the densest cell's centre. `None` when no cell is above.
fn dense_cells(plan: &StitchPlan, cell_mm: f64, max: f64) -> Option<(usize, f64, Pt)> {
    let pts: Vec<Pt> =
        plan.blocks.iter().flat_map(|b| b.steps.iter()).filter_map(|s| if let Step::Stitch(p) = s { Some(*p) } else { None }).collect();
    let first = pts.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in &pts {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    let span = f64::from(x1 - x0).max(f64::from(y1 - y0));
    let cell = (cell_mm * UNITS_PER_MM).max(span / (MAX_CELLS_SIDE - 1) as f64).max(1.0);
    // At most MAX_CELLS_SIDE by the choice of cell.
    let side = |lo: i32, hi: i32| ((f64::from(hi - lo) / cell).floor() as usize + 1).min(MAX_CELLS_SIDE);
    let (cols, rows) = (side(x0, x1), side(y0, y1));
    let mut counts = vec![0u32; cols * rows];
    for p in &pts {
        let c = ((f64::from(p.x - x0) / cell).floor() as usize).min(cols - 1);
        let r = ((f64::from(p.y - y0) / cell).floor() as usize).min(rows - 1);
        if let Some(n) = counts.get_mut(r * cols + c) {
            *n = n.saturating_add(1);
        }
    }
    let area = (cell / UNITS_PER_MM).powi(2);
    let (mut over, mut worst, mut at) = (0usize, 0.0f64, 0usize);
    for (i, n) in counts.iter().enumerate() {
        let d = f64::from(*n) / area;
        if d > max {
            over += 1;
        }
        if d > worst {
            (worst, at) = (d, i);
        }
    }
    if over == 0 {
        return None;
    }
    let (c, r) = (at % cols, at / cols);
    // Within the design's extents, so the casts are exact.
    let centre = Pt { x: x0 + ((c as f64 + 0.5) * cell) as i32, y: y0 + ((r as f64 + 0.5) * cell) as i32 };
    Some((over, worst, centre))
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::{BAI_MIRROR, Block, GENERIC, Thread};

    fn S(x: i32, y: i32) -> Step {
        Step::Stitch(Pt { x, y })
    }
    fn block(rgb: [u8; 3], steps: Vec<Step>) -> Block {
        Block { thread: Thread { rgb, ..Default::default() }, steps }
    }
    fn codes(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|f| f.code).collect()
    }

    #[test]
    fn an_empty_plan_is_an_error() {
        let f = check_plan(&StitchPlan::default(), &GENERIC, &CheckLimits::default());
        assert_eq!(codes(&f), ["empty"]);
        assert_eq!(f[0].level, Level::Error);
    }

    #[test]
    fn short_long_and_over_the_machine() {
        // 2 mm stitches, one of 0.2 mm, one of 9 mm, one of 15 mm.
        let p = StitchPlan { name: String::new(), blocks: vec![block([0; 3], vec![S(0, 0), S(20, 0), S(22, 0), S(112, 0), S(262, 0)])] };
        let f = check_plan(&p, &GENERIC, &CheckLimits::default());
        assert_eq!(codes(&f), ["overMaxStitch", "shortStitches", "longStitches", "colors"]);
        assert_eq!(f[1].count, Some(1));
        assert_eq!(f[1].at, Some(Pt { x: 22, y: 0 }));
        assert_eq!(f[2].count, Some(2));
        assert_eq!(f[0].count, Some(1));
        assert_eq!(f[3].level, Level::Info);
    }

    #[test]
    fn density_trims_and_colours_past_the_needles() {
        // 40 needle points in one 2 mm cell: 10 per mm².
        let steps: Vec<Step> = (0..40).map(|i| S(5 + (i % 2) * 6, 5)).collect();
        let blocks: Vec<Block> = (0..16u8).map(|i| block([i, 0, 0], if i == 0 { steps.clone() } else { vec![S(100, 100), S(130, 100)] })).collect();
        let mut p = StitchPlan { name: String::new(), blocks };
        if let Some(b) = p.blocks.get_mut(1) {
            b.steps.extend([Step::Trim, Step::Trim]);
        }
        let l = CheckLimits { max_trims: 1, ..CheckLimits::default() };
        let f = check_plan(&p, &BAI_MIRROR, &l);
        assert_eq!(codes(&f), ["density", "trims", "colors"], "{f:?}");
        assert_eq!(f[0].count, Some(1));
        assert_eq!(f[0].at, Some(Pt { x: 15, y: 15 }));
        assert_eq!((f[1].count, f[2].count, f[2].level), (Some(2), Some(16), Level::Warning));
        // Raising the threshold clears it.
        let calm = check_plan(&p, &BAI_MIRROR, &CheckLimits { max_density: 20.0, max_trims: 5, ..l });
        assert_eq!(codes(&calm), ["colors"]);
    }
}
