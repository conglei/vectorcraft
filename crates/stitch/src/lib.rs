//! VectorCraft embroidery: the stitch plan and what is built on it.
//!
//! A [`StitchPlan`] is the machine-neutral result of preparing a design: thread blocks of
//! steps in 0.1 mm units, y down.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod calibration;
mod fill;
mod from_doc;
mod plan;
mod prepare;
mod preview;
mod profile;
mod run;
mod satin;
mod stats;
mod thread;
mod to_doc;

#[cfg(test)]
mod tests_props;

pub use calibration::{CalibrationPart, calibration};
pub use fill::{FillParams, MAX_ROWS, TOO_MANY_ROWS, tatami};
pub use from_doc::{
    DEFAULT_SATIN_WIDTH_MM, FillOptions, FillType, FromDocOptions, Mode, Planned, SATIN_WIDTH_MM, StrokeOptions, StrokeType, from_document,
};
pub use plan::{Block, MAX_STEPS, PT_PER_MM, Pt, Step, StitchPlan, TOO_MANY_STEPS, Thread, UNITS_PER_MM};
pub use prepare::{LOCK_LENGTH, PrepareOptions, Prepared, prepare};
pub use preview::{
    DEFAULT_FABRIC, DEFAULT_SCALE, DEFAULT_THREAD_MM, DENSITY_CELL_MM, DensityGrid, MAX_DENSITY_CELLS, MAX_SCALE, MAX_SIDE, MAX_THREAD_MM, MIN_SCALE,
    MIN_THREAD_MM, Preview, PreviewMode, PreviewOptions, TOO_DENSE, density, preview, preview_size,
};
pub use profile::{BAI_MIRROR, COMMERCIAL15, GENERIC, MachineProfile, profile};
pub use run::run_stitches;
pub use satin::{SatinParams, satin};
pub use stats::{Stats, ThreadStats, stats};
pub use thread::{
    JSON_FORMAT, MAX_BRAND, MAX_CHART_BYTES, MAX_CHART_NAME, MAX_CHART_THREADS, MAX_CODE, MAX_NAME, MAX_WEIGHT, NeedleUse, Needles, STARTER_BRAND,
    STARTER_NAME, ThreadChart, ThreadMatch, ThreadSpec, assign_needles, delta_e, hex, lab, starter_chart,
};
pub use to_doc::{THREAD_WIDTH_MM, to_document};
