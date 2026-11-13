//! Making a stitch plan safe for a machine.

use crate::profile::MachineProfile;
use crate::stats::stats;
use crate::{Block, Pt, Step, StitchPlan, Thread};

/// Length of a lock stitch, in units (0.6 mm).
pub const LOCK_LENGTH: i32 = 6;

/// What [`prepare`] does besides the machine's limits.
#[derive(Clone, Copy, Debug)]
pub struct PrepareOptions {
    /// Add lock stitches at the start and end of each run.
    pub locks: bool,
    /// Add a trim before travel long enough that the machine would read it as one (see
    /// [`MachineProfile::jumps_per_trim`]). Trims already in the plan stay either way.
    pub travel_trims: bool,
}

impl Default for PrepareOptions {
    fn default() -> Self {
        PrepareOptions { locks: true, travel_trims: true }
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

/// The `i`-th of `k` equal steps from `a` to `b`, rounded to whole units: `i == k` is `b`
/// itself. Consecutive points differ by at most `ceil(d / k)` per axis, `d` the larger axis
/// distance, and by the same amount to within one unit.
fn between(a: Pt, b: Pt, i: i64, k: i64) -> Pt {
    let at = |from: i32, to: i32| -> i32 {
        let delta = f64::from(to) - f64::from(from);
        // Between `from` and `to`, so it fits an i32.
        (f64::from(from) + (delta * i as f64 / k.max(1) as f64).round()) as i32
    };
    Pt { x: at(a.x, b.x), y: at(a.y, b.y) }
}

/// Whether [`split_long_jumps`] sews the last part of the jump from `a` to `b` as the stitch that
/// follows it, `next`: the jump needs splitting, `next` is a stitch at `b`, and the last part is
/// no longer than `max_stitch`. Travel then ends in a stitch rather than in a jump and a stitch
/// in place: one jump record fewer.
fn folds(a: Pt, b: Pt, next: Option<&Step>, p: &MachineProfile) -> bool {
    let d = cheb(a, b);
    if d <= i64::from(p.max_jump) || next != Some(&Step::Stitch(b)) {
        return false;
    }
    let k = records(d, p.max_jump);
    cheb(between(a, b, k - 1, k), b) <= i64::from(p.max_stitch)
}

fn position(step: &Step) -> Option<Pt> {
    match step {
        Step::Stitch(p) | Step::Jump(p) => Some(*p),
        Step::Trim | Step::Stop => None,
    }
}

/// A block's steps, each with a tag: what made it (an object's place in the sewing order, for
/// [`prepare_tagged`]). A step a pass adds takes the tag of the step it is added for.
#[derive(Clone, Debug, Default)]
struct Tagged {
    thread: Thread,
    steps: Vec<Step>,
    tags: Vec<u32>,
}

impl Tagged {
    fn push(&mut self, s: Step, t: u32) {
        self.steps.push(s);
        self.tags.push(t);
    }
}

/// The tag of step `i` (0 past the tags given).
fn tag(tags: &[u32], i: usize) -> u32 {
    tags.get(i).copied().unwrap_or(0)
}

fn steps_of(plan: &[Tagged]) -> usize {
    plan.iter().map(|b| b.steps.len()).sum()
}

fn within_limit(plan: &[Tagged]) -> Result<(), String> {
    if steps_of(plan) > crate::MAX_STEPS {
        return Err(crate::TOO_MANY_STEPS.to_string());
    }
    Ok(())
}

/// Applies `f` to every block's steps and tags, giving it the position the needle is at on entry
/// and updating that position afterwards.
fn map_blocks(plan: &[Tagged], mut f: impl FnMut(&[Step], &[u32], &mut Option<Pt>) -> Result<Tagged, String>) -> Result<Vec<Tagged>, String> {
    let mut pos = None;
    let mut blocks = Vec::with_capacity(plan.len());
    for b in plan {
        let mut out = f(&b.steps, &b.tags, &mut pos)?;
        out.thread = b.thread.clone();
        blocks.push(out);
    }
    Ok(blocks)
}

/// Pass 1: drop stitches closer than `min_stitch` to the last kept point of their run, except
/// the last stitch of the run.
fn drop_small_stitches(plan: &[Tagged], p: &MachineProfile) -> Vec<Tagged> {
    let min = f64::from(p.min_stitch);
    plan.iter()
        .map(|b| {
            let mut out = Tagged { thread: b.thread.clone(), steps: Vec::with_capacity(b.steps.len()), tags: Vec::with_capacity(b.steps.len()) };
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
                out.push(*s, tag(&b.tags, i));
            }
            out
        })
        .collect()
}

/// Pass 4: a stitch longer than `max_stitch` becomes `k = ceil(d / max)` equal pieces, `d` its
/// larger axis distance and `max` the smaller of `max_stitch` and `max_jump`: `k − 1` jumps and a
/// final stitch that ends exactly on the stitch's end point. The pieces differ by at most one unit
/// per axis (see [`between`]), so none is longer than `max` and no stitch has zero length. A split
/// stitch is not travel: it gets no trim and no lock stitches (the thread stays attached).
/// Returns the plan and how many stitches needed `jumps_per_trim` or more jumps, which a machine
/// that trims after that many jump records cuts.
fn split_long_stitches(plan: &[Tagged], p: &MachineProfile) -> Result<(Vec<Tagged>, usize), String> {
    let max = p.max_stitch.min(p.max_jump).max(1);
    let mut count = steps_of(plan);
    let mut cut = 0;
    let plan = map_blocks(plan, |steps, tags, pos| {
        let mut out = Tagged::default();
        for (n, s) in steps.iter().enumerate() {
            let t = tag(tags, n);
            if let Step::Stitch(b) = s
                && let Some(a) = *pos
                && cheb(a, *b) > i64::from(max)
            {
                let k = records(cheb(a, *b), max);
                count = count.saturating_add(usize::try_from(k - 1).unwrap_or(usize::MAX));
                if count > crate::MAX_STEPS {
                    return Err(crate::TOO_MANY_STEPS.to_string());
                }
                if p.jumps_per_trim > 0 && k > i64::from(p.jumps_per_trim) {
                    cut += 1;
                }
                for i in 1..k {
                    out.push(Step::Jump(between(a, *b, i, k)), t);
                }
            }
            if let Some(pt) = position(s) {
                *pos = Some(pt);
            }
            out.push(*s, t);
        }
        Ok(out)
    })?;
    Ok((plan, cut))
}

/// Pass 3: a travel (consecutive jumps) that needs `jumps_per_trim` or more jump records gets a
/// `Trim` before it, unless one is already there, nothing has been sewn yet, or it starts a
/// block (the colour change already cut the thread). It runs after the lock stitches, so travel is
/// measured from where the needle really is, and before long stitches are split, which are not
/// travel on their own. Records are counted as passes 4 and 5 write them: when travel ends in a
/// stitch in place and its last part folds into that stitch ([`folds`]), that part is not a jump;
/// when it ends in a long stitch, that stitch's jumps continue the same run.
fn trim_long_travel(plan: &[Tagged], p: &MachineProfile) -> Result<Vec<Tagged>, String> {
    let mut stitched = false;
    let mut prev_trim = false;
    map_blocks(plan, |steps, tags, pos| {
        // A block boundary counts as a trim.
        prev_trim = true;
        let mut out = Tagged::default();
        let mut i = 0;
        while let Some(s) = steps.get(i) {
            if let Step::Jump(_) = s {
                let end = steps.iter().skip(i).take_while(|s| matches!(s, Step::Jump(_))).count() + i;
                let mut at = *pos;
                let mut total = 0i64;
                let mut last = None;
                for j in steps.get(i..end).unwrap_or_default() {
                    if let Step::Jump(pt) = j {
                        total += at.map_or(1, |q| records(cheb(q, *pt), p.max_jump));
                        last = at.map(|q| (q, *pt));
                        at = Some(*pt);
                    }
                }
                if let Some((a, b)) = last
                    && folds(a, b, steps.get(end), p)
                {
                    total -= 1;
                }
                // A long stitch after the travel adds its jumps to the same run of records.
                let max = p.max_stitch.min(p.max_jump).max(1);
                if let (Some(a), Some(Step::Stitch(b))) = (at, steps.get(end))
                    && cheb(a, *b) > i64::from(max)
                {
                    total += records(cheb(a, *b), max) - 1;
                }
                if p.jumps_per_trim > 0 && total >= i64::from(p.jumps_per_trim) && stitched && !prev_trim {
                    out.push(Step::Trim, tag(tags, i));
                }
                for (k, j) in steps.get(i..end).unwrap_or_default().iter().enumerate() {
                    out.push(*j, tag(tags, i + k));
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
            out.push(*s, tag(tags, i));
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

/// Pass 2: three back-and-forth stitches at each end of every run of two or more stitches.
fn add_locks(plan: &[Tagged]) -> Vec<Tagged> {
    plan.iter()
        .map(|b| {
            let mut out = locked(&b.steps, &b.tags);
            out.thread = b.thread.clone();
            out
        })
        .collect()
}

/// `steps` with lock stitches at both ends of each run of two or more stitches; a lock takes the
/// tag of the stitch it locks.
fn locked(steps: &[Step], tags: &[u32]) -> Tagged {
    let mut out = Tagged { thread: Thread::default(), steps: Vec::with_capacity(steps.len()), tags: Vec::with_capacity(steps.len()) };
    let mut i = 0;
    while let Some(s) = steps.get(i) {
        let Step::Stitch(_) = s else {
            out.push(*s, tag(tags, i));
            i += 1;
            continue;
        };
        let end = steps.iter().skip(i).take_while(|s| matches!(s, Step::Stitch(_))).count() + i;
        let pts: Vec<(Pt, u32)> =
            steps.get(i..end).unwrap_or_default().iter().enumerate().filter_map(|(k, s)| position(s).map(|p| (p, tag(tags, i + k)))).collect();
        let lock = pts.len() >= 2;
        for (n, (pt, t)) in pts.iter().enumerate() {
            out.push(Step::Stitch(*pt), *t);
            if n == 0
                && lock
                && let Some((next, _)) = pts.get(1)
            {
                let q = toward(*pt, *next, LOCK_LENGTH);
                for s in [Step::Stitch(q), Step::Stitch(*pt), Step::Stitch(q)] {
                    out.push(s, *t);
                }
            }
        }
        if lock && let [.., (prev, _), (last, t)] = pts.as_slice() {
            let q = toward(*last, *prev, LOCK_LENGTH);
            for s in [Step::Stitch(q), Step::Stitch(*last), Step::Stitch(q)] {
                out.push(s, *t);
            }
        }
        i = end;
    }
    out
}

/// Pass 5: split jumps longer than `max_jump` into equal parts. When the travel ends in a stitch
/// in place, its last part is sewn by that stitch ([`folds`]): `k − 1` jumps and a stitch, not `k`
/// jumps and a stitch of zero length. The pieces of a split stitch (pass 4) are no longer than
/// `max_jump` and are left alone.
fn split_long_jumps(plan: &[Tagged], p: &MachineProfile) -> Result<Vec<Tagged>, String> {
    let mut count = steps_of(plan);
    map_blocks(plan, |steps, tags, pos| {
        let mut out = Tagged::default();
        for (n, s) in steps.iter().enumerate() {
            let t = tag(tags, n);
            if let (Step::Jump(b), Some(a)) = (s, *pos) {
                let d = cheb(a, *b);
                if d > i64::from(p.max_jump) {
                    let k = records(d, p.max_jump);
                    count = count.saturating_add(usize::try_from(k - 1).unwrap_or(usize::MAX));
                    if count > crate::MAX_STEPS {
                        return Err(crate::TOO_MANY_STEPS.to_string());
                    }
                    for i in 1..k {
                        out.push(Step::Jump(between(a, *b, i, k)), t);
                    }
                    if folds(a, *b, steps.get(n + 1), p) {
                        *pos = Some(*b);
                        continue;
                    }
                }
            }
            if let Some(pt) = position(s) {
                *pos = Some(pt);
            }
            out.push(*s, t);
        }
        Ok(out)
    })
}

/// Pass 6: what the machine cannot do, as warnings. `cut`: how many stitches were split into
/// `jumps_per_trim` or more jumps.
fn check(plan: &StitchPlan, p: &MachineProfile, cut: usize) -> Vec<String> {
    let mut warnings = Vec::new();
    if cut > 0 {
        let t = p.jumps_per_trim;
        let mm = f64::from(i32::from(t).saturating_mul(p.max_stitch.min(p.max_jump))) / crate::UNITS_PER_MM;
        let what = if cut == 1 { "1 stitch is".to_string() } else { format!("{cut} stitches are") };
        warnings.push(format!("{what} longer than {mm:.1} mm: split into {t} or more jumps, which a machine that trims after {t} jumps cuts"));
    }
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

/// Makes `plan` safe for `profile`: small stitches dropped, locks added, long travel trimmed
/// (measured after the locks), long stitches and jumps split, and what remains wrong reported as
/// warnings.
pub fn prepare(plan: &StitchPlan, profile: &MachineProfile, o: &PrepareOptions) -> Result<Prepared, String> {
    prepare_tagged(plan, &[], profile, o).map(|(p, _)| p)
}

/// [`prepare`], with a tag for each step of `plan` (`tags[block][step]`; a step without one is
/// tagged 0), returned for each step of the prepared plan: a step [`prepare`] adds (a lock stitch,
/// a trim before travel, a piece of a split stitch or jump) takes the tag of the step it is added
/// for. The tags say which object made each step ([`crate::Planned::owners`]).
pub fn prepare_tagged(plan: &StitchPlan, tags: &[Vec<u32>], profile: &MachineProfile, o: &PrepareOptions) -> Result<(Prepared, Vec<Vec<u32>>), String> {
    plan.check()?;
    let input: Vec<Tagged> = plan
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let given = tags.get(i).map(Vec::as_slice).unwrap_or_default();
            Tagged { thread: b.thread.clone(), steps: b.steps.clone(), tags: (0..b.steps.len()).map(|k| tag(given, k)).collect() }
        })
        .collect();
    let t = drop_small_stitches(&input, profile);
    let t = if o.locks { add_locks(&t) } else { t };
    within_limit(&t)?;
    let t = if o.travel_trims { trim_long_travel(&t, profile)? } else { t };
    within_limit(&t)?;
    let (t, cut) = split_long_stitches(&t, profile)?;
    let t = split_long_jumps(&t, profile)?;
    let mut blocks = Vec::with_capacity(t.len());
    let mut out_tags = Vec::with_capacity(t.len());
    for b in t {
        blocks.push(Block { thread: b.thread, steps: b.steps });
        out_tags.push(b.tags);
    }
    let plan = StitchPlan { name: plan.name.clone(), blocks };
    let warnings = check(&plan, profile, cut);
    Ok((Prepared { plan, warnings }, out_tags))
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
        let p = prepare(&plan(vec![S(0, 0), S(1, 0), S(30, 0), S(31, 0)]), &GENERIC, &PrepareOptions { locks: false, ..Default::default() })
            .unwrap()
            .plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(30, 0), S(31, 0)]);
    }

    #[test]
    fn a_long_stitch_becomes_jumps_then_a_stitch() {
        let p = prepare(&plan(vec![S(0, 0), S(200, 0)]), &GENERIC, &PrepareOptions { locks: false, ..Default::default() }).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), J(100, 0), S(200, 0)]);
    }

    #[test]
    fn a_stitch_long_enough_to_read_as_a_trim_is_reported() {
        let o = PrepareOptions { locks: false, ..Default::default() };
        // 36.3 mm: two jumps and a stitch. 36.4 mm: three jumps.
        assert!(prepare(&plan(vec![S(0, 0), S(363, 0)]), &GENERIC, &o).unwrap().warnings.is_empty());
        let p = prepare(&plan(vec![S(0, 0), S(364, 0), S(0, 0)]), &GENERIC, &o).unwrap();
        assert!(!p.plan.blocks[0].steps.contains(&Step::Trim));
        assert_eq!(p.warnings, vec!["2 stitches are longer than 36.3 mm: split into 3 or more jumps, which a machine that trims after 3 jumps cuts"]);
    }

    #[test]
    fn travel_ending_in_a_stitch_in_place_sews_its_last_part() {
        let o = PrepareOptions { locks: false, travel_trims: false };
        let p = prepare(&plan(vec![S(0, 0), J(300, 0), S(300, 0), S(330, 0)]), &GENERIC, &o).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), J(100, 0), J(200, 0), S(300, 0), S(330, 0)]);
        // A last part longer than the longest stitch stays a jump.
        let short = MachineProfile { max_stitch: 50, ..GENERIC };
        let p = prepare(&plan(vec![S(0, 0), J(300, 0), S(300, 0), S(330, 0)]), &short, &o).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), J(100, 0), J(200, 0), J(300, 0), S(300, 0), S(330, 0)]);
    }

    #[test]
    fn a_30_mm_stitch_is_two_jumps_and_a_stitch_with_no_trim() {
        for profile in [&GENERIC, &COMMERCIAL15] {
            for locks in [false, true] {
                let p = prepare(&plan(vec![S(0, 0), S(300, 0)]), profile, &PrepareOptions { locks, ..Default::default() }).unwrap().plan;
                let steps = &p.blocks[0].steps;
                assert!(!steps.contains(&Step::Trim), "{}: {steps:?}", profile.id);
                let i = steps.iter().position(|s| matches!(s, Step::Jump(_))).expect("the first jump");
                // The start lock leaves the needle at 6.
                let pieces = if locks { [J(104, 0), J(202, 0), S(300, 0)] } else { [J(100, 0), J(200, 0), S(300, 0)] };
                assert_eq!(steps[i..i + 3], pieces, "{}: {steps:?}", profile.id);
            }
        }
    }

    #[test]
    fn travel_of_three_jump_records_is_trimmed_first() {
        let o = PrepareOptions { locks: false, ..Default::default() };
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), J(400, 0), S(400, 0), S(430, 0)]), &GENERIC, &o).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(30, 0), Step::Trim, J(123, 0), J(215, 0), J(308, 0), S(400, 0), S(430, 0)]);
        // 27 mm of travel ending in a stitch is two jumps and that stitch: no trim.
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), J(300, 0), S(300, 0), S(330, 0)]), &GENERIC, &o).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(30, 0), J(120, 0), J(210, 0), S(300, 0), S(330, 0)]);
    }

    #[test]
    fn travel_is_measured_from_where_the_lock_stitches_leave_the_needle() {
        // Before locks, 30 -> 390 is 360 units: two jumps and the stitch. The end lock leaves the
        // needle at 24, and 24 -> 390 is 366 units: three jumps, which the machine reads as a trim.
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), J(390, 0), S(390, 0), S(420, 0)]), &GENERIC, &PrepareOptions::default()).unwrap().plan;
        let steps = &p.blocks[0].steps;
        let trim = steps.iter().position(|s| *s == Step::Trim).expect("a trim before the travel");
        assert_eq!(steps.get(trim - 1), Some(&S(24, 0)), "{steps:?}");
        assert_eq!(steps[trim + 1..trim + 5], [J(116, 0), J(207, 0), J(299, 0), S(390, 0)], "{steps:?}");
    }

    #[test]
    fn travel_trims_can_be_turned_off_and_the_plans_own_trims_stay() {
        let steps = vec![S(0, 0), S(30, 0), Step::Trim, J(120, 0), S(120, 0), S(150, 0), J(550, 0), S(550, 0), S(580, 0)];
        let o = PrepareOptions { locks: false, travel_trims: false };
        let p = prepare(&plan(steps.clone()), &GENERIC, &o).unwrap().plan;
        assert_eq!(p.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 1, "{:?}", p.blocks[0].steps);
        let p = prepare(&plan(steps), &GENERIC, &PrepareOptions { locks: false, ..Default::default() }).unwrap().plan;
        assert_eq!(p.blocks[0].steps.iter().filter(|s| **s == Step::Trim).count(), 2, "{:?}", p.blocks[0].steps);
    }

    #[test]
    fn travel_at_the_start_of_a_colour_is_not_trimmed_again() {
        let mut p = plan(vec![S(0, 0), S(30, 0)]);
        p.blocks.push(Block { thread: Thread { rgb: [1, 2, 3], ..Default::default() }, steps: vec![J(400, 0), S(400, 0), S(430, 0)] });
        let out = prepare(&p, &GENERIC, &PrepareOptions { locks: false, ..Default::default() }).unwrap().plan;
        assert_eq!(out.blocks[1].steps, vec![J(123, 0), J(215, 0), J(308, 0), S(400, 0), S(430, 0)]);
    }

    #[test]
    fn runs_are_locked_at_both_ends() {
        let p = prepare(&plan(vec![S(0, 0), S(30, 0), S(60, 0)]), &GENERIC, &PrepareOptions::default()).unwrap().plan;
        assert_eq!(p.blocks[0].steps, vec![S(0, 0), S(6, 0), S(0, 0), S(6, 0), S(30, 0), S(60, 0), S(54, 0), S(60, 0), S(54, 0)]);
    }

    #[test]
    fn needle_and_field_warnings() {
        let mut many = StitchPlan::default();
        for i in 0..16u8 {
            many.blocks.push(Block { thread: Thread { rgb: [i, 0, 0], ..Default::default() }, steps: vec![S(0, 0), S(6000, 0)] });
        }
        let w = prepare(&many, &COMMERCIAL15, &PrepareOptions { locks: false, ..Default::default() }).unwrap().warnings;
        assert!(w.contains(&"16 thread colours, but the machine has 15 needles".to_string()));
        assert!(w.contains(&"the design is 600.0 × 0.0 mm, larger than the 500.0 × 350.0 mm field".to_string()));
    }

    #[test]
    fn an_empty_block_is_reported_by_its_one_based_number() {
        let mut p = plan(vec![S(0, 0), S(30, 0)]);
        p.blocks.push(Block::default());
        let w = prepare(&p, &GENERIC, &PrepareOptions { locks: false, ..Default::default() }).unwrap().warnings;
        assert_eq!(w, vec!["colour 2 has no stitches".to_string()]);
    }

    #[test]
    fn a_plan_that_grows_past_the_step_limit_is_refused() {
        let p = plan(vec![S(0, 0), J(2_000_000_000, 0), S(2_000_000_000, 0)]);
        assert!(prepare(&p, &GENERIC, &PrepareOptions { locks: false, ..Default::default() }).is_err());
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
        fn a_long_stitch_is_equal_jumps_and_one_stitch(
            x0 in -5000i32..5000, y0 in -5000i32..5000, len in 122i64..5000, t in 0.0f64..1.0, sign in any::<(bool, bool)>(),
        ) {
            // A point at Chebyshev distance `len` from (x0, y0), in any direction.
            let other = (len as f64 * t).round() as i32;
            let (mut dx, mut dy) = if t < 0.5 { (len as i32, other) } else { (other, len as i32) };
            if sign.0 { dx = -dx; }
            if sign.1 { dy = -dy; }
            let (a, b) = (Pt { x: x0, y: y0 }, Pt { x: x0 + dx, y: y0 + dy });
            let o = PrepareOptions { locks: false, ..Default::default() };
            let out = prepare(&plan(vec![Step::Stitch(a), Step::Stitch(b)]), &GENERIC, &o).unwrap().plan;
            let steps = &out.blocks[0].steps;
            let max = i64::from(GENERIC.max_stitch);
            let k = (len + max - 1) / max;
            prop_assert_eq!(steps.len() as i64, k + 1, "{:?}", steps);
            prop_assert_eq!(steps.first(), Some(&Step::Stitch(a)));
            prop_assert_eq!(steps.last(), Some(&Step::Stitch(b)));
            let mut at = a;
            for s in &steps[1..] {
                let (Step::Jump(p) | Step::Stitch(p)) = *s else { return Err(TestCaseError::fail(format!("{s:?}"))) };
                prop_assert!(cheb(at, p) <= max, "{:?} is {} from {:?}", s, cheb(at, p), at);
                at = p;
            }
            prop_assert!(steps[1..steps.len() - 1].iter().all(|s| matches!(s, Step::Jump(_))), "{:?}", steps);
        }

        #[test]
        fn prepared_plans_respect_the_machine(
            blocks in proptest::collection::vec(proptest::collection::vec(step(), 0..40), 0..=3),
            locks in any::<bool>(),
        ) {
            let input = StitchPlan {
                name: String::new(),
                blocks: blocks.into_iter().map(|steps| Block { thread: Thread::default(), steps }).collect(),
            };
            let out = prepare(&input, &GENERIC, &PrepareOptions { locks, ..Default::default() }).unwrap().plan;
            let (max, n) = (GENERIC.max_jump, i64::from(GENERIC.jumps_per_trim));
            // The true needle position, whether anything has been sewn, and whether the thread
            // was just cut (by a trim or a colour change).
            let mut pos: Option<Pt> = None;
            let mut stitched = false;
            for b in &out.blocks {
                let mut cut = true;
                // Jump records in the current run of jumps, whether a cut precedes the run, and the
                // smallest and largest move of the run on each axis.
                let mut run = 0i64;
                let mut announced = true;
                let mut moves: Option<[i64; 4]> = None;
                for s in &b.steps {
                    // A run that needs a trim and has none must be a split stitch: equal pieces
                    // (to within a unit per axis) ending in this stitch.
                    let unannounced = run >= n && !announced;
                    match *s {
                        Step::Stitch(p) => {
                            if let Some(q) = pos {
                                prop_assert!(cheb(q, p) <= i64::from(GENERIC.max_stitch), "stitch {s:?} is {} from {q:?}", cheb(q, p));
                                if unannounced {
                                    let (dx, dy) = (i64::from(p.x) - i64::from(q.x), i64::from(p.y) - i64::from(q.y));
                                    let [x0, x1, y0, y1] = moves.unwrap_or([dx, dx, dy, dy]);
                                    let equal = x0.min(dx) + 1 >= x1.max(dx) && y0.min(dy) + 1 >= y1.max(dy);
                                    prop_assert!(equal, "{} jump records without a trim: {:?}", run, b.steps);
                                }
                            }
                            pos = Some(p);
                            stitched = true;
                            run = 0;
                            moves = None;
                            cut = false;
                        }
                        Step::Jump(p) => {
                            if run == 0 {
                                announced = cut || !stitched;
                            }
                            // Records the writer needs: ceil(larger axis distance / max_jump), at least one.
                            let r = pos.map_or(1, |q| ((cheb(q, p) + i64::from(max) - 1) / i64::from(max)).max(1));
                            prop_assert_eq!(r, 1, "jump {:?} from {:?} needs {} records", s, pos, r);
                            if let Some(q) = pos {
                                let (dx, dy) = (i64::from(p.x) - i64::from(q.x), i64::from(p.y) - i64::from(q.y));
                                let [x0, x1, y0, y1] = moves.unwrap_or([dx, dx, dy, dy]);
                                moves = Some([x0.min(dx), x1.max(dx), y0.min(dy), y1.max(dy)]);
                            }
                            run += r;
                            pos = Some(p);
                            cut = false;
                        }
                        Step::Trim | Step::Stop => {
                            prop_assert!(!unannounced, "{} jump records without a trim: {:?}", run, b.steps);
                            run = 0;
                            moves = None;
                            cut = matches!(s, Step::Trim);
                        }
                    }
                }
                prop_assert!(run < n || announced, "{} jump records without a trim: {:?}", run, b.steps);
            }
        }
    }
}
