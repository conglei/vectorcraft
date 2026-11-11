//! The stitch plan data model.

/// Plan units per millimetre: coordinates are tenths of a millimetre.
pub const UNITS_PER_MM: f64 = 10.0;
/// The most steps a plan may hold.
pub const MAX_STEPS: usize = 2_000_000;
/// The error for a plan past [`MAX_STEPS`].
pub const TOO_MANY_STEPS: &str = "the design has more than 2000000 stitches";
/// Document points per millimetre.
pub const PT_PER_MM: f64 = 72.0 / 25.4;

/// Largest coordinate magnitude, in units, that a point may take.
const LIMIT: f64 = 10_000_000.0;
/// Millimetres per document point.
pub(crate) const MM_PER_PT: f64 = 25.4 / 72.0;

/// A position in plan units (0.1 mm), y down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pt {
    pub x: i32,
    pub y: i32,
}

/// One machine action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Sew to the point.
    Stitch(Pt),
    /// Move to the point without sewing.
    Jump(Pt),
    /// Cut the thread.
    Trim,
    /// Pause the machine without changing thread, e.g. to place appliqué fabric. DST and EXP
    /// write it as the same record as a colour change.
    Stop,
}

/// A thread: its colour and optional name and catalogue code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thread {
    pub rgb: [u8; 3],
    pub name: Option<String>,
    pub code: Option<String>,
}

/// The steps sewn with one thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub thread: Thread,
    pub steps: Vec<Step>,
}

/// A whole design: blocks in sewing order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StitchPlan {
    pub name: String,
    pub blocks: Vec<Block>,
}

fn unit(v: f64) -> Option<i32> {
    if !v.is_finite() {
        return None;
    }
    let r = v.round();
    if r.abs() > LIMIT {
        return None;
    }
    // Finite and within +-10,000,000, so the cast is exact.
    Some(r as i32)
}

impl Pt {
    /// The point at (`x`, `y`) millimetres, rounded to the nearest unit. `None` if either is
    /// not finite or lies outside +-10,000,000 units.
    pub fn from_mm(x: f64, y: f64) -> Option<Pt> {
        Some(Pt { x: unit(x * UNITS_PER_MM)?, y: unit(y * UNITS_PER_MM)? })
    }

    /// The point at (`x`, `y`) document points, rounded to the nearest unit. `None` as for
    /// [`Pt::from_mm`].
    pub fn from_pt(x: f64, y: f64) -> Option<Pt> {
        Pt::from_mm(x * MM_PER_PT, y * MM_PER_PT)
    }

    /// The point in millimetres.
    pub fn to_mm(self) -> (f64, f64) {
        (f64::from(self.x) / UNITS_PER_MM, f64::from(self.y) / UNITS_PER_MM)
    }

    /// The distance to `o`, in units.
    pub fn dist(self, o: Pt) -> f64 {
        let dx = f64::from(self.x) - f64::from(o.x);
        let dy = f64::from(self.y) - f64::from(o.y);
        dx.hypot(dy)
    }
}

impl StitchPlan {
    /// The number of steps over all blocks.
    pub fn steps(&self) -> usize {
        self.blocks.iter().map(|b| b.steps.len()).sum()
    }

    /// Whether the plan is within the step limit.
    pub fn check(&self) -> Result<(), String> {
        if self.steps() > MAX_STEPS {
            return Err(TOO_MANY_STEPS.to_string());
        }
        Ok(())
    }

    /// The position of every stitch and jump, in order.
    pub fn points(&self) -> impl Iterator<Item = Pt> + '_ {
        self.blocks.iter().flat_map(|b| b.steps.iter()).filter_map(|s| match s {
            Step::Stitch(p) | Step::Jump(p) => Some(*p),
            Step::Trim | Step::Stop => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_round_to_tenths_of_a_millimetre() {
        assert_eq!(Pt::from_mm(1.04, -2.06), Some(Pt { x: 10, y: -21 }));
        assert_eq!(Pt::from_pt(72.0, 36.0), Some(Pt { x: 254, y: 127 }));
        assert_eq!(Pt::from_mm(f64::NAN, 0.0), None);
        assert_eq!(Pt::from_mm(1.0e9, 0.0), None);
    }

    #[test]
    fn a_plan_counts_its_steps_and_refuses_too_many() {
        let block = Block { thread: Thread::default(), steps: vec![Step::Stitch(Pt::default()); 3] };
        let plan = StitchPlan { name: "t".into(), blocks: vec![block.clone(), block] };
        assert_eq!(plan.steps(), 6);
        assert!(plan.check().is_ok());
        let big = StitchPlan { name: "t".into(), blocks: vec![Block { thread: Thread::default(), steps: vec![Step::Trim; MAX_STEPS + 1] }] };
        assert_eq!(big.check().unwrap_err(), "the design has more than 2000000 stitches");
        assert_eq!(TOO_MANY_STEPS, format!("the design has more than {MAX_STEPS} stitches"));
    }
}
