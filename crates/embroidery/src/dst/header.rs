//! The DST header: 512 bytes of ASCII fields, each a label, a colon, a fixed-width value and
//! a carriage return, then 0x1A and spaces.

use vectorcraft_stitch::Thread;

/// The header's length.
pub(crate) const LEN: usize = 512;
const NAME_WIDTH: usize = 16;

/// The extents of the design from its origin, in file orientation (y up). Each is zero or
/// more: `min_x` and `min_y` are distances to the left and below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extents {
    pub max_x: i64,
    pub min_x: i64,
    pub max_y: i64,
    pub min_y: i64,
}

/// What a reader takes from the header.
pub(crate) struct Fields {
    pub name: String,
    pub threads: Vec<Thread>,
}

/// `v` clamped to what `width` digits can show.
pub(crate) fn fit(v: i64, width: u32) -> i64 {
    v.clamp(0, 10i64.pow(width) - 1)
}

/// A signed field: the sign, then a five-wide magnitude.
pub(crate) fn signed(v: i64) -> String {
    let sign = if v < 0 { '-' } else { '+' };
    format!("{sign}{:>5}", fit(v.abs(), 5))
}

/// Warnings for counts too large for their header fields, which then show their largest value.
pub(crate) fn overflow(records: usize, color_changes: usize) -> Vec<String> {
    let mut w = Vec::new();
    if records > 9_999_999 {
        w.push("the stitch count is too large for the file header".to_string());
    }
    if color_changes > 999 {
        w.push("the colour change count is too large for the file header".to_string());
    }
    w
}

/// A design name as the `LA:` field holds it: letters, digits, spaces, `-` and `_`, at most 16.
pub(crate) fn label(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(NAME_WIDTH).collect()
}

/// The header for a file of `records` records (including the end record) with
/// `color_changes` colour changes, the given extents and end point (relative to the origin,
/// y up).
pub(crate) fn write(name: &str, records: usize, color_changes: usize, ext: Extents, end: (i32, i32)) -> [u8; LEN] {
    let name = label(name);
    let records = fit(i64::try_from(records).unwrap_or(i64::MAX), 7);
    let color_changes = fit(i64::try_from(color_changes).unwrap_or(i64::MAX), 3);
    let text = format!(
        "LA:{name:<16}\rST:{records:>7}\rCO:{color_changes:>3}\r+X:{:>5}\r-X:{:>5}\r+Y:{:>5}\r-Y:{:>5}\rAX:{}\rAY:{}\rMX:+{:>5}\rMY:+{:>5}\rPD:******\r\x1a",
        fit(ext.max_x, 5),
        fit(ext.min_x, 5),
        fit(ext.max_y, 5),
        fit(ext.min_y, 5),
        signed(i64::from(end.0)),
        signed(i64::from(end.1)),
        0,
        0,
    );
    let mut out = [b' '; LEN];
    for (o, c) in out.iter_mut().zip(text.bytes()) {
        *o = c;
    }
    out
}

/// A thread from a `TC:` value: `#RRGGBB`, then an optional name and code, separated by
/// commas.
fn thread(value: &str) -> Option<Thread> {
    let mut parts = value.split(',').map(str::trim);
    let hex = parts.next()?.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    let [_, r, g, b] = n.to_be_bytes();
    let text = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_string);
    let name = text(parts.next());
    let code = text(parts.next());
    Some(Thread { rgb: [r, g, b], name, code, brand: None })
}

/// The name and thread colours from a header. Counts and extents are not trusted, so they
/// are not read.
pub(crate) fn read(h: &[u8]) -> Fields {
    let h = h.get(..LEN).unwrap_or(h);
    let text: String = h.iter().take_while(|c| **c != 0x1A).map(|c| if c.is_ascii() { char::from(*c) } else { ' ' }).collect();
    let mut fields = Fields { name: String::new(), threads: Vec::new() };
    for line in text.split(['\r', '\n']) {
        if let Some(v) = line.strip_prefix("LA:") {
            fields.name = v.chars().filter(|c| !c.is_ascii_control()).collect::<String>().trim().to_string();
        } else if let Some(t) = line.strip_prefix("TC:").and_then(thread) {
            fields.threads.push(t);
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_large_values_keep_the_field_widths() {
        let ext = Extents { max_x: 123_456, min_x: 7, max_y: 0, min_y: 0 };
        let h = write("A", 12_345_678, 1000, ext, (-3, 200_000));
        let text = String::from_utf8(h.to_vec()).unwrap();
        assert!(text.starts_with("LA:A               \rST:9999999\rCO:999\r+X:99999\r-X:    7\r"));
        assert!(text.contains("\rAX:-    3\rAY:+99999\rMX:+    0\r"));
        assert_eq!(h.iter().position(|c| *c == 0x1A), Some(124));
    }

    #[test]
    fn the_reader_takes_the_name_and_threads() {
        let f = read(b"LA:Rose          \rST:      1\rTC:#FF0000,Red,R1\rTC:#00ff00\rTC:junk\r\x1aTC:#0000FF\r");
        assert_eq!(f.name, "Rose");
        assert_eq!(f.threads.len(), 2);
        assert_eq!(f.threads[0], Thread { rgb: [255, 0, 0], name: Some("Red".into()), code: Some("R1".into()), brand: None });
        assert_eq!(f.threads[1], Thread { rgb: [0, 255, 0], name: None, code: None, brand: None });
    }
}
