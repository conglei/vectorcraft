//! Making a stitch plan safe for a machine.

use crate::profile::MachineProfile;
use crate::stats::stats;
use crate::{Block, Pt, Step, StitchPlan};

/// Length of a lock stitch, in units (0.6 mm).
pub const LOCK_LENGTH: i32 = 6;

/// What [`prepare`] does besides the machine's limits.
#[derive(Clone, Copy, Debug)]
pub struct PrepareOptions {
    /// Add lock stitches at the start and end of each run.
    pub locks: bool,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        PrepareOptions { locks: true }
    }
}

/// A plan made safe for a machine, with what could not be fixed.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub plan: StitchPlan,
    pub warnings: Vec<String>,
}

/// The larger of the two axis distances.
fn cheb(a: Pt, b: Pt) -> i64 {
    (i64::from(a.x) - i64::from(b.x)).abs().max((i64::from(a.y) - i64::from(b.y)).abs())
}

/// How many jump records of at most `max` units cover `d` units (at least one).
fn records(d: i64, max: i32) -> i64 {
    let m = i64::from(max.max(1));
    ((d + m - 1) / m).max(1)
}

fn position(step: &Step) -> Option<Pt> {
    match step {
        Step::Stitch(p) | Step::Jump(p) => Some(*p),
        Step::Trim | Step::Stop => None,
    }
}

/// Applies `f` to every block's steps, giving it the position the needle is at on entry and
/// updating that position afterwards.
fn map_blocks(plan: &StitchPlan, mut f: impl FnMut(&[Step], &mut Option<Pt>) -> Result<Vec<Step>, String>) -> Result<StitchPlan, String> {
    let mut pos = None;
    let mut blocks = Vec::with_capacity(plan.blocks.len());
    for b in &plan.blocks {
        let steps = f(&b.steps, &mut pos)?;
        blocks.push(Block { thread: b.thread.clone(), steps });
    }
    Ok(StitchPlan { name: plan.name.clone(), blocks })
}

/// Pass 1: drop stitches closer than `min_stitch` to the last kept point of their run, except
/// the last stitch of the run.
fn drop_small_stitches(plan: &StitchPlan, p: &MachineProfile) -> StitchPlan {
    let min = f64::from(p.min_stitch);
    let blocks = plan
        .blocks
        .iter()
        .map(|b| {
            let mut out = Vec::with_capacity(b.steps.len());
            let mut kept: Option<Pt> = None;
            for (i, s) in b.steps.iter().enumerate() {
                if let Step::Stitch(pt) = s {
                    let last = !matches!(b.steps.get(i + 1), Some(Step::Stitch(_)));
                    if let Some(k) = kept
                        && !last
                        && pt.dist(k) < min
                    {
                        continue;
                    }
                    kept = Some(*pt);
                } else {
                    kept = None;
                }
                out.push(*s);
            }
            Block { thread: b.thread.clone(), steps: out }
        })
        .collect();
    StitchPlan { name: plan.name.clone(), blocks }
}

/// Pass 2: a stitch longer than `max_stitch` becomes a jump to its end point and a stitch.
fn split_long_stitches(plan: &StitchPlan, p: &MachineProfile) -> Result<StitchPlan, String> {
    map_blocks(plan, |steps, pos| {
        let mut out = Vec::with_capacity(steps.len());
        for s in steps {
            if let Step::Stitch(pt) = s
                && let Some(q) = *pos
                && cheb(q, *pt) > i64::from(p.max_stitch)
            {
                out.push(Step::Jump(*pt));
            }
            if let Some(pt) = position(s) {
                *pos = Some(pt);
            }
            out.push(*s);
        }
        Ok(out)
    })
}

/// Pass 4: a travel (consecutive jumps) that needs `jumps_per_trim` or more records gets a
/// `Trim` before it, unless one is already there, nothing has been sewn yet, or it starts a
/// block (the colour change already cut the thread). It runs after the lock stitches, so travel is
/// measured from where the needle really is.
fn trim_long_travel(plan: &StitchPlan, p: &MachineProfile) -> Result<StitchPlan, String> {
    let mut stitched = false;
    let mut prev_trim = false;
    map_blocks(plan, |steps, pos| {
        // A block boundary counts as a trim.
        prev_trim = true;
        let mut out = Vec::with_capacity(steps.len());
        let mut i = 0;
        while let Some(s) = steps.get(i) {
            if let Step::Jump(_) = s {
                let end = steps.iter().skip(i).take_while(|s| matches!(s, Step::Jump(_))).count() + i;
                let mut at = *pos;
                let mut total = 0i64;
                for j in steps.get(i..end).unwrap_or_default() {
                    if let Step::Jump(pt) = j {
                        total += at.map_or(1, |q| records(cheb(q, *pt), p.max_jump));
                        at = Some(*pt);
                    }
                }
                if p.jumps_per_trim > 0 && total >= i64::from(p.jumps_per_trim) && stitched && !prev_trim {
                    out.push(Step::Trim);
                }
                for j in steps.get(i..end).unwrap_or_default() {
                    out.push(*j);
                }
                *pos = at;
                prev_trim = false;
                i = end;
                continue;
            }
            if let Step::Stitch(pt) = s {
                stitched = true;
                *pos = Some(*pt);
            }
            prev_trim = matches!(s, Step::Trim);
            out.push(*s);
            i += 1;
        }
        Ok(out)
    })
}

/// The point `len` units from `from` toward `to`, or `to` if it is nearer than that.
fn toward(from: Pt, to: Pt, len: i32) -> Pt {
    let d = from.dist(to);
    if d <= f64::from(len) {
        return to;
    }
    let k = f64::from(len) / d;
    let dx = f64::from(to.x) - f64::from(from.x);
    let dy = f64::from(to.y) - f64::from(from.y);
    // |offset| <= len, so the casts are exact and the sums stay within i32.
    Pt { x: from.x.saturating_add((dx * k).round() as i32), y: from.y.saturating_add((dy * k).round() as i32) }
}

/// Pass 3: three back-and-forth stitches at each end of every run of two or more stitches.
fn add_locks(plan: &StitchPlan) -> StitchPlan {
    let blocks = plan.blocks.iter().map(|b| Block { thread: b.thread.clone(), steps: locked(&b.steps) }).collect();
    StitchPlan { name: plan.name.clone(), blocks }
}

/// `steps` with lock stitches at both ends of each run of two or more stitches.
fn locked(steps: &[Step]) -> Vec<Step> {
    let mut out = Vec::with_capacity(steps.len());
    let mut i = 0;
    while let Some(s) = steps.get(i) {
        let Step::Stitch(_) = s else {
            out.push(*s);
            i += 1;
            continue;
        };
        let end = steps.iter().skip(i).take_while(|s| matches!(s, Step::Stitch(_))).count() + i;
        let pts: Vec<Pt> = steps.get(i..end).unwrap_or_default().iter().filter_map(position).collect();
        let lock = pts.len() >= 2;
        for (n, pt) in pts.iter().enumerate() {
            out.push(Step::Stitch(*pt));
            if n == 0
                && lock
                && let Some(next) = pts.get(1)
            {
                let q = toward(*pt, *next, LOCK_LENGTH);
                out.extend([Step::Stitch(q), Step::Stitch(*pt), Step::Stitch(q)]);
            }
        }
        if lock && let [.., prev, last] = pts.as_slice() {
            let q = toward(*last, *prev, LOCK_LENGTH);
            out.extend([Step::Stitch(q), Step::Stitch(*last), Step::Stitch(q)]);
        }
        i = end;
    }
    out
}

/// Pass 5: split jumps longer than `max_jump` into equal parts.
fn split_long_jumps(plan: &StitchPlan, p: &MachineProfile) -> Result<StitchPlan, String> {
    let mut count = plan.steps();
    map_blocks(plan, |steps, pos| {
        let mut out = Vec::with_capacity(steps.len());
        for s in steps {
            if let (Step::Jump(b), Some(a)) = (s, *pos) {
                let d = cheb(a, *b);
                if d > i64::from(p.max_jump) {
                    let k = records(d, p.max_jump);
                    count = count.saturating_add(usize::try_from(k - 1).unwrap_or(usize::MAX));
                    if count > crate::MAX_STEPS {
                        return Err(format!("the design has more than {} stitches", crate::MAX_STEPS));
                    }
                    for i in 1..k {
                        let at = |from: i32, to: i32| -> i32 {
                            let delta = f64::from(to) - f64::from(from);
                            // Between `from` and `to`, so it fits an i32.
                            (f64::from(from) + (delta * i as f64 / k as f64).round()) as i32
                        };
                        out.push(Step::Jump(Pt { x: at(a.x, b.x), y: at(a.y, b.y) }));
                    }
                }
            }
            if let Some(pt) = position(s) {
                *pos = Some(pt);
            }
            out.push(*s);
        }
        Ok(out)
    })
}

/// Pass 6: what the machine cannot do, as warnings.
fn check(plan: &StitchPlan, p: &MachineProfile) -> Vec<String> {
    let mut warnings = Vec::new();
    let st = stats(plan);
    if let Some(n) = p.needles
        && st.threads.len() > usize::from(n)
    {
        warnings.push(format!("{} thread colours, but the machine has {} needles", st.threads.len(), n));
    }
    if let Some((fw, fh)) = p.field_mm
        && (st.width_mm > fw || st.height_mm > fh)
    {
        warnings.push(format!("the design is {:.1} × {:.1} mm, larger than the {:.1} × {:.1} mm field", st.width_mm, st.height_mm, fw, fh));
    }
    for (i, b) in plan.blocks.iter().enumerate() {
        if !b.steps.iter().any(|s| matches!(s, Step::Stitch(_))) {
            warnings.push(format!("colour {} has no stitches", i + 1));
        }
    }
    warnings
}

/// Makes `plan` safe for `profile`: small stitches dropped, long stitches and jumps split,
/// locks added, long travel trimmed (measured after the locks), and what remains wrong reported as warnings.
pub fn prepare(plan: &StitchPlan, profile: &MachineProfile, o: &PrepareOptions) -> Result<Prepared, String> {
    plan.check()?;
    let plan = drop_small_stitches(plan, profile);
    let plan = split_long_stitches(&plan, profile)?;
    plan.check()?;
    let plan = if o.locks { add_locks(&plan) } else { plan };
    plan.check()?;
    let plan = trim_long_travel(&plan, profile)?;
    plan.check()?;
    let plan = split_long_jumps(&plan, profile)?;
    let warnings = check(&plan, profile);
    Ok(Prepared { plan, warnings })
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::profile::{COMMERCIAL15, GENERIC};
    use crate::{Block, Pt, Step, StitchPlan, Thread};
    use proptest::prelude::*;

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
    fn small_stitches_are_dropped_but_not_the_last() {
        let p = prepare(&plan(vec![S(0, 0), S(1, 0), S(30, 0), S(31, 0)]), &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(30, 0), S(31, 0)]);
    }

    #[test]
    fn a_long_stitch_becomes_jumps_then_a_stitch() {
        let p = prepare(&plan(vec![S(0, 0), S(200, 0)]), &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), J(100, 0), J(200, 0), S(200, 0)]);
    }

    #[test]
    fn travel_of_three_jump_records_is_trimmed_first() {
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), J(300, 0), S(300, 0), S(330, 0)]), &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(30, 0), Step::Trim, J(120, 0), J(210, 0), J(300, 0), S(300, 0), S(330, 0)]);
    }

    #[test]
    fn travel_is_measured_from_where_the_lock_stitches_leave_the_needle() {
        // Before locks, 30 -> 272 is 242 units: two records. The end lock leaves the needle at 24,
        // and 24 -> 272 is 248 units: three records, which the machine reads as a trim.
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), J(272, 0), S(272, 0), S(300, 0)]), &GENERIC, &PrepareOptions { locks: true }).unwrap().plan;
        let steps = &p.blocks[0].steps;
        let trim = steps.iter().position(|s| *s == Step::Trim).expect("a trim before the travel");
        assert_eq!(steps.get(trim - 1), Some(&S(24, 0)), "{steps:?}");
        assert_eq!(steps[trim + 1..trim + 4], [J(107, 0), J(189, 0), J(272, 0)], "{steps:?}");
    }

    #[test]
    fn travel_at_the_start_of_a_colour_is_not_trimmed_again() {
        let mut p = plan(vec![S(0, 0), S(30, 0)]);
        p.blocks.push(Block { thread: Thread { rgb: [1, 2, 3], ..Default::default() }, steps: vec![J(400, 0), S(400, 0), S(430, 0)] });
        let out = prepare(&p, &GENERIC, &PrepareOptions { locks: false }).unwrap().plan;
        assert_eq!(out.blocks[1].steps, vec![J(123, 0), J(215, 0), J(308, 0), J(400, 0), S(400, 0), S(430, 0)]);
    }

    #[test]
    fn runs_are_locked_at_both_ends() {
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), S(60, 0)]), &GENERIC, &PrepareOptions { locks: true }).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(6, 0), S(0, 0), S(6, 0), S(30, 0), S(60, 0), S(54, 0), S(60, 0), S(54, 0)]);
    }

    #[test]
    fn needle_and_field_warnings() {
        let mut many = StitchPlan::default();
        for i in 0..16u8 {
            many.blocks.push(Block { thread: Thread { rgb: [i, 0, 0], ..Default::default() }, steps: vec![S(0, 0), S(6000, 0)] });
        }
        let w = prepare(&many, &COMMERCIAL15, &PrepareOptions { locks: false }).unwrap().warnings;
        assert!(w.contains(&"16 thread colours, but the machine has 15 needles".to_string()));
        assert!(w.contains(&"the design is 600.0 × 0.0 mm, larger than the 500.0 × 350.0 mm field".to_string()));
    }

    #[test]
    fn an_empty_block_is_reported_by_its_one_based_number() {
        let mut p = plan(vec![S(0, 0), S(30, 0)]);
        p.blocks.push(Block::default());
        let w = prepare(&p, &GENERIC, &PrepareOptions { locks: false }).unwrap().warnings;
        assert_eq!(w, vec!["colour 2 has no stitches".to_string()]);
    }

    #[test]
    fn a_plan_that_grows_past_the_step_limit_is_refused() {
        let p = plan(vec![S(0, 0), J(2_000_000_000, 0), S(2_000_000_000, 0)]);
        assert!(prepare(&p, &GENERIC, &PrepareOptions { locks: false }).is_err());
    }

    fn step() -> impl Strategy<Value = Step> {
        // Small coordinates too, so travel often lands near a record boundary.
        let c = prop_oneof![-5000i32..=5000, -300i32..=300];
        prop_oneof![
            4 => (c.clone(), c.clone()).prop_map(|(x, y)| Step::Stitch(Pt { x, y })),
            2 => (c.clone(), c).prop_map(|(x, y)| Step::Jump(Pt { x, y })),
            1 => Just(Step::Trim),
            1 => Just(Step::Stop),
        ]
    }

    proptest! {
        #[test]
        fn prepared_plans_respect_the_machine(
            blocks in proptest::collection::vec(proptest::collection::vec(step(), 0..40), 0..=3),
            locks in any::<bool>(),
        ) {
            let input = StitchPlan {
                name: String::new(),
                blocks: blocks.into_iter().map(|steps| Block { thread: Thread::default(), steps }).collect(),
            };
            let out = prepare(&input, &GENERIC, &PrepareOptions { locks }).unwrap().plan;
            let (max, n) = (GENERIC.max_jump, i64::from(GENERIC.jumps_per_trim));
            // The true needle position, whether anything has been sewn, and whether the thread
            // was just cut (by a trim or a colour change).
            let mut pos: Option<Pt> = None;
            let mut stitched = false;
            for b in &out.blocks {
                let mut cut = true;
                // Jump records in the current run of jumps, and whether a cut precedes the run.
                let mut run = 0i64;
                let mut announced = true;
                for s in &b.steps {
                    match *s {
                        Step::Stitch(p) => {
                            if let Some(q) = pos {
                                prop_assert!(cheb(q, p) <= i64::from(GENERIC.max_stitch), "stitch {s:?} is {} from {q:?}", cheb(q, p));
                            }
                            pos = Some(p);
                            stitched = true;
                            run = 0;
                            cut = false;
                        }
                        Step::Jump(p) => {
                            if run == 0 {
                                announced = cut || !stitched;
                            }
                            // Records the writer needs: ceil(larger axis distance / max_jump), at least one.
                            let r = pos.map_or(1, |q| ((cheb(q, p) + i64::from(max) - 1) / i64::from(max)).max(1));
                            prop_assert_eq!(r, 1, "jump {:?} from {:?} needs {} records", s, pos, r);
                            run += r;
                            prop_assert!(run < n || announced, "{} jump records without a trim: {:?}", run, b.steps);
                            pos = Some(p);
                            cut = false;
                        }
                        Step::Trim => {
                            run = 0;
                            cut = true;
                        }
                        Step::Stop => {
                            run = 0;
                            cut = false;
                        }
                    }
                }
            }
        }
    }
}
