//! DST stitch records: three bytes, a move of up to 121 units per axis written as a sum of
//! +-1, +-3, +-9, +-27 and +-81, plus flags in the third byte.

/// The largest move per axis.
pub(crate) const MAX_MOVE: i32 = 121;
/// Flags for a stitch record.
pub(crate) const STITCH: u8 = 0x00;
/// Flags for a jump record.
pub(crate) const JUMP: u8 = 0x80;
/// A colour change or stop.
pub(crate) const COLOR_CHANGE: [u8; 3] = [0, 0, 0xC3];
/// The end of the file.
pub(crate) const END: [u8; 3] = [0, 0, 0xF3];

/// Each weight, with the byte that holds it and its plus and minus bits.
const X_BITS: [(i32, usize, u8, u8); 5] = [(1, 0, 0x01, 0x02), (3, 1, 0x01, 0x02), (9, 0, 0x04, 0x08), (27, 1, 0x04, 0x08), (81, 2, 0x04, 0x08)];
const Y_BITS: [(i32, usize, u8, u8); 5] = [(1, 0, 0x80, 0x40), (3, 1, 0x80, 0x40), (9, 0, 0x20, 0x10), (27, 1, 0x20, 0x10), (81, 2, 0x20, 0x10)];

/// Sets the bits for `v` (within +-121) in balanced ternary.
fn put(b: &mut [u8; 3], mut v: i32, table: &[(i32, usize, u8, u8); 5]) {
    for &(_, byte, plus, minus) in table {
        let bit = match v.rem_euclid(3) {
            1 => {
                v -= 1;
                plus
            }
            2 => {
                v += 1;
                minus
            }
            _ => 0,
        };
        v /= 3;
        if let Some(x) = b.get_mut(byte) {
            *x |= bit;
        }
    }
}

/// The record for a move of (`dx`, `dy`), y up, with `flags` ([`STITCH`] or [`JUMP`]).
/// `None` outside +-121.
pub(crate) fn encode(dx: i32, dy: i32, flags: u8) -> Option<[u8; 3]> {
    if dx.abs() > MAX_MOVE || dy.abs() > MAX_MOVE {
        return None;
    }
    let mut b = [0, 0, 0x03 | flags];
    put(&mut b, dx, &X_BITS);
    put(&mut b, dy, &Y_BITS);
    Some(b)
}

/// The move a record makes, y up.
pub(crate) fn decode(b: [u8; 3]) -> (i32, i32) {
    let sum = |table: &[(i32, usize, u8, u8); 5]| {
        table.iter().fold(0, |acc, &(w, byte, plus, minus)| {
            let x = b.get(byte).copied().unwrap_or(0);
            acc + if x & plus != 0 { w } else { 0 } - if x & minus != 0 { w } else { 0 }
        })
    };
    (sum(&X_BITS), sum(&Y_BITS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_moves_have_known_bytes() {
        assert_eq!(encode(1, 0, 0), Some([0x01, 0x00, 0x03]));
        assert_eq!(encode(0, 1, 0), Some([0x80, 0x00, 0x03]));
        assert_eq!(encode(5, 0, 0), Some([0x06, 0x02, 0x03]));
        assert_eq!(encode(121, -121, 0), Some([0x55, 0x55, 0x17]));
        assert_eq!(encode(2, 0, 0x80), Some([0x02, 0x01, 0x83]));
        assert_eq!(encode(122, 0, 0), None);
    }

    #[test]
    fn every_move_round_trips() {
        for dx in -121..=121 {
            for dy in -121..=121 {
                assert_eq!(decode(encode(dx, dy, 0).unwrap()), (dx, dy));
            }
        }
    }
}
