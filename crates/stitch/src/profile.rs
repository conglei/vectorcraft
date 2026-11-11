//! Machine profiles: the limits a stitch plan must respect for a machine.

/// What a machine can do, in plan units (0.1 mm).
#[derive(Clone, Debug, PartialEq)]
pub struct MachineProfile {
    pub id: &'static str,
    /// The longest stitch, per axis.
    pub max_stitch: i32,
    /// The longest single jump record, per axis.
    pub max_jump: i32,
    /// Stitches closer than this to the last needle point are dropped.
    pub min_stitch: i32,
    /// Number of needles, if the machine has a fixed number.
    pub needles: Option<u8>,
    /// Jump records in a row that make the machine cut the thread. 0 turns travel trims off.
    pub jumps_per_trim: u8,
    /// The embroidery field (width, height) in mm, if limited.
    pub field_mm: Option<(f64, f64)>,
}

/// A machine with no needle or field limits.
pub const GENERIC: MachineProfile =
    MachineProfile { id: "generic", max_stitch: 121, max_jump: 121, min_stitch: 3, needles: None, jumps_per_trim: 3, field_mm: None };

/// A 15-needle commercial machine with a 500 x 350 mm field.
pub const COMMERCIAL15: MachineProfile = MachineProfile {
    id: "commercial15",
    max_stitch: 121,
    max_jump: 121,
    min_stitch: 3,
    needles: Some(15),
    jumps_per_trim: 3,
    field_mm: Some((500.0, 350.0)),
};

/// The built-in profile with this id.
pub fn profile(id: &str) -> Option<MachineProfile> {
    match id {
        "generic" => Some(GENERIC),
        "commercial15" => Some(COMMERCIAL15),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_profiles_are_found_by_id() {
        assert_eq!(profile("generic"), Some(GENERIC));
        assert_eq!(profile("commercial15").and_then(|p| p.needles), Some(15));
        assert_eq!(profile("nope"), None);
    }
}
