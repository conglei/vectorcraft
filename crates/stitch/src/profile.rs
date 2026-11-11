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
    /// The most stitches per minute the machine sews on flat goods, for run-time estimates.
    /// `None` when not known.
    pub max_spm: Option<u32>,
    /// Ids of the file formats the machine's panel reads (`dst`, `dsb`). Empty when not known.
    pub formats: &'static [&'static str],
}

/// A machine with no needle or field limits.
pub const GENERIC: MachineProfile = MachineProfile {
    id: "generic",
    max_stitch: 121,
    max_jump: 121,
    min_stitch: 3,
    needles: None,
    jumps_per_trim: 3,
    field_mm: None,
    max_spm: None,
    formats: &[],
};

/// A 15-needle commercial machine with a 500 x 350 mm field.
pub const COMMERCIAL15: MachineProfile = MachineProfile {
    id: "commercial15",
    max_stitch: 121,
    max_jump: 121,
    min_stitch: 3,
    needles: Some(15),
    jumps_per_trim: 3,
    field_mm: Some((500.0, 350.0)),
    max_spm: None,
    formats: &[],
};

/// The owner's machine: a commercial single-head 15-needle machine, from public reseller pages
/// (see `docs/embroidery-parity.md`, Machine facts). The limits are `COMMERCIAL15`'s: the field is
/// the majority figure (one page says 500 x 800 mm, unverified) and the longest stitch and the
/// jumps that make a trim are the DST defaults, not yet confirmed on the machine.
pub const BAI_MIRROR: MachineProfile = MachineProfile {
    id: "bai_mirror",
    max_stitch: 121,
    max_jump: 121,
    min_stitch: 3,
    needles: Some(15),
    jumps_per_trim: 3,
    field_mm: Some((500.0, 350.0)),
    // 1,200 stitches per minute flat; 850 on caps.
    max_spm: Some(1200),
    // The panel reads DST, and DSB on some pages; not EXP, PES or JEF.
    formats: &["dst", "dsb"],
};

/// The built-in profile with this id.
pub fn profile(id: &str) -> Option<MachineProfile> {
    match id {
        "generic" => Some(GENERIC),
        "commercial15" => Some(COMMERCIAL15),
        "bai_mirror" => Some(BAI_MIRROR),
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

    #[test]
    fn bai_mirror_has_the_commercial15_limits_and_adds_speed_and_formats() {
        let p = profile("bai_mirror").unwrap();
        assert_eq!(p, BAI_MIRROR);
        let c = COMMERCIAL15;
        assert_eq!(
            (p.max_stitch, p.max_jump, p.min_stitch, p.needles, p.jumps_per_trim, p.field_mm),
            (c.max_stitch, c.max_jump, c.min_stitch, c.needles, c.jumps_per_trim, c.field_mm)
        );
        assert_eq!((p.max_spm, p.formats), (Some(1200), &["dst", "dsb"][..]));
        assert_eq!((GENERIC.max_spm, GENERIC.formats), (None, &[][..]));
    }
}
