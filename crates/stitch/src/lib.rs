//! VectorCraft embroidery: the stitch plan and what is built on it.
//!
//! A [`StitchPlan`] is the machine-neutral result of preparing a design: thread blocks of
//! steps in 0.1 mm units, y down.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod plan;

pub use plan::{Block, MAX_STEPS, Pt, Step, StitchPlan, Thread, UNITS_PER_MM};
