//! VectorCraft embroidery: the stitch plan and what is built on it.
//!
//! A [`StitchPlan`] is the machine-neutral result of preparing a design: thread blocks of
//! steps in 0.1 mm units, y down.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod autodigitize;
mod calibration;
mod check;
mod columns;
mod fill;
mod fill_field;
mod from_doc;
pub mod hoop;
mod lettering;
mod plan;
mod prepare;
mod preview;
mod profile;
mod props;
mod run;
mod satin;
pub mod sequence;
mod skeleton;
mod stats;
mod thread;
mod to_doc;
pub mod travel;

#[cfg(test)]
mod tests_props;

pub use autodigitize::{AutoKind, AutoParams, AutoShape, COMPACT, FILL_OVERLAP_MM, STUB_FACTOR, StrokeKind, auto_shape, stroke_kind};
pub use calibration::{CalibrationPart, calibration};
pub use check::{CELL_RANGE, CheckLimits, DENSITY_RANGE, Finding, LONG_RANGE, Level, MAX_TRIMS_LIMIT, SHORT_RANGE, check_plan};
pub use fill::{FillParams, MAX_ROWS, TOO_MANY_ROWS, tatami};
pub use fill_field::{DirectionGrid, FeatherEdges, FieldFillParams, MAX_GRID_CELLS, TOO_LARGE_FIELD_FILL, field_fill};
pub use from_doc::{
    DEFAULT_SATIN_WIDTH_MM, FillOptions, FillType, FlatLine, FromDocOptions, Lettered, Mode, Planned, SATIN_WIDTH_MM, SequenceItem, Sequenced,
    StrokeOptions, StrokeType, closest_join_ways, colour_sort_order, covered_fills, from_document, lettering_of, lettering_params, letters_of,
    outline_fraction, outline_mm, outline_point, sequence, text_mode,
};
pub use hoop::{DEFAULT_MARGIN_MM, Hoop, HoopShape, MAX_MARGIN_MM, Placement};
pub use lettering::{
    COMPENSATION_MM, Column, Letter, LetterMode, LetterShape, LetteringParams, MAX_LETTERS, MAX_SATIN_MM, MIN_COLUMN_MM, NO_TRIM_GAP_MM, PieceLetter,
    PieceStyle, RUN_LENGTH_MM, RUN_UNDER_MM, SewnLetter, UNDERLAY_LENGTH_MM, letter_height, mode_for, sew_letters, sew_pieces, shape_letter,
};
pub use plan::{Block, MAX_STEPS, PT_PER_MM, Pt, Step, StitchPlan, TOO_MANY_STEPS, Thread, UNITS_PER_MM};
pub use prepare::{LOCK_LENGTH, PrepareOptions, Prepared, prepare, prepare_tagged};
pub use preview::{
    DEFAULT_FABRIC, DEFAULT_SCALE, DEFAULT_THREAD_MM, DENSITY_CELL_MM, DensityGrid, MARGIN_MM, MAX_DENSITY_CELLS, MAX_SCALE, MAX_SIDE, MAX_THREAD_MM,
    MIN_REGION_SCALE, MIN_SCALE, MIN_THREAD_MM, Preview, PreviewMode, PreviewOptions, Region, TOO_DENSE, density, prefix_plan, preview,
    preview_prefix, preview_region, preview_size,
};
pub use profile::{BAI_MIRROR, COMMERCIAL15, GENERIC, MachineProfile, profile};
pub use props::{effective_number, inherited_props, sewn_as};
pub use run::run_stitches;
pub use satin::{EDGE_WALK_FROM_MM, RailsParams, SatinParams, ZIGZAG_FROM_MM, ZIGZAG_SPACING_MM, rails_satin, satin, underlay_for};
pub use stats::{Stats, TagStats, ThreadStats, stats, stats_by_tag};
pub use thread::{
    JSON_FORMAT, MAX_BRAND, MAX_CHART_BYTES, MAX_CHART_NAME, MAX_CHART_THREADS, MAX_CODE, MAX_NAME, MAX_WEIGHT, NeedleUse, Needles, STARTER_BRAND,
    STARTER_NAME, ThreadChart, ThreadMatch, ThreadSpec, assign_needles, delta_e, hex, lab, starter_chart,
};
pub use to_doc::{THREAD_WIDTH_MM, to_document};
