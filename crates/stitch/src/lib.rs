//! VectorCraft embroidery: the stitch plan and what is built on it.
//!
//! A [`StitchPlan`] is the machine-neutral result of preparing a design: thread blocks of
//! steps in 0.1 mm units, y down.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod plan;
mod prepare;
mod profile;
mod stats;

pub use plan::{Block, MAX_STEPS, Pt, Step, StitchPlan, Thread, UNITS_PER_MM};
pub use prepare::{LOCK_LENGTH, PrepareOptions, Prepared, prepare};
pub use profile::{COMMERCIAL15, GENERIC, MachineProfile, profile};
pub use stats::{Stats, ThreadStats, stats};
