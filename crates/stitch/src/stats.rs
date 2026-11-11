//! Counts and extents of a stitch plan.

use crate::{Pt, Step, StitchPlan, UNITS_PER_MM};

/// Per thread colour: its stitch count and stitched length.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadStats {
    pub rgb: [u8; 3],
    pub stitches: usize,
    pub length_mm: f64,
}

/// What a plan contains.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub stitches: usize,
    pub jumps: usize,
    pub trims: usize,
    pub color_changes: usize,
    pub width_mm: f64,
    pub height_mm: f64,
    /// One entry per distinct colour, in order of first use.
    pub threads: Vec<ThreadStats>,
}

/// Counts steps, measures the extents of the stitches and sums stitched length per colour.
pub fn stats(plan: &StitchPlan) -> Stats {
    let (mut stitches, mut jumps, mut trims) = (0usize, 0usize, 0usize);
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    let mut threads: Vec<ThreadStats> = Vec::new();
    // Stitched length per thread, in units, summed before converting to mm.
    let mut units: Vec<f64> = Vec::new();
    for b in &plan.blocks {
        let idx = match threads.iter().position(|t| t.rgb == b.thread.rgb) {
            Some(i) => i,
            None => {
                threads.push(ThreadStats { rgb: b.thread.rgb, stitches: 0, length_mm: 0.0 });
                units.push(0.0);
                threads.len() - 1
            }
        };
        let mut prev: Option<Pt> = None;
        for s in &b.steps {
            match s {
                Step::Stitch(p) => {
                    stitches += 1;
                    if let Some(t) = threads.get_mut(idx) {
                        t.stitches += 1;
                    }
                    if let (Some(q), Some(u)) = (prev, units.get_mut(idx)) {
                        *u += q.dist(*p);
                    }
                    bounds = Some(match bounds {
                        None => (p.x, p.y, p.x, p.y),
                        Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
                    });
                    prev = Some(*p);
                    continue;
                }
                Step::Jump(_) => jumps += 1,
                Step::Trim => trims += 1,
                Step::Stop => {}
            }
            prev = None;
        }
    }
    for (t, u) in threads.iter_mut().zip(&units) {
        t.length_mm = u / UNITS_PER_MM;
    }
    let (width_mm, height_mm) = bounds.map_or((0.0, 0.0), |(x0, y0, x1, y1)| (f64::from(x1) - f64::from(x0), f64::from(y1) - f64::from(y0)));
    Stats {
        stitches,
        jumps,
        trims,
        color_changes: plan.blocks.len().saturating_sub(1),
        width_mm: width_mm / UNITS_PER_MM,
        height_mm: height_mm / UNITS_PER_MM,
        threads,
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::{Block, Pt, Step, StitchPlan, Thread};

    fn S(x: i32, y: i32) -> Step {
        Step::Stitch(Pt { x, y })
    }
    fn J(x: i32, y: i32) -> Step {
        Step::Jump(Pt { x, y })
    }
    fn plan(steps: Vec<Step>) -> StitchPlan {
        StitchPlan { name: String::new(), blocks: vec![Block { thread: Thread::default(), steps }] }
    }

    #[test]
    fn stats_count_what_is_there() {
        let s = stats(&plan(vec![S(0, 0), S(30, 0), Step::Trim, J(100, 40), S(100, 40), S(100, 70)]));
        assert_eq!((s.stitches, s.jumps, s.trims, s.color_changes), (4, 1, 1, 0));
        assert_eq!((s.width_mm, s.height_mm), (10.0, 7.0));
        assert_eq!(s.threads, vec![ThreadStats { rgb: [0, 0, 0], stitches: 4, length_mm: 6.0 }]);
    }

    #[test]
    fn blocks_of_one_colour_merge_and_an_empty_plan_has_no_changes() {
        let mut p = plan(vec![S(0, 0), S(10, 0)]);
        p.blocks.push(Block { thread: Thread { rgb: [1, 2, 3], ..Default::default() }, steps: vec![S(0, 0)] });
        p.blocks.push(Block { thread: Thread::default(), steps: vec![S(0, 10), S(0, 20)] });
        let s = stats(&p);
        assert_eq!(s.color_changes, 2);
        assert_eq!(s.threads.len(), 2);
        assert_eq!(s.threads[0], ThreadStats { rgb: [0, 0, 0], stitches: 4, length_mm: 2.0 });
        let e = stats(&StitchPlan::default());
        assert_eq!((e.color_changes, e.width_mm, e.height_mm), (0, 0.0, 0.0));
    }
}
