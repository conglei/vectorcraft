//! The PEC thread palette: the 64 colours a PEC file's colour bytes index (1 to 64; 0 is none),
//! with their names. The catalogue number of entry `n` is `n`.
//!
//! The table is data from pyembroidery 1.5.1, `EmbThreadPec.py` (MIT License, copyright (c) 2018
//! the pyembroidery authors, https://github.com/EmbroidePy/pyembroidery); see NOTICE. The same
//! table is published at https://edutechwiki.unige.ch/en/Embroidery_format_PEC.

use vectorcraft_stitch::Thread;

/// Entries 1 to 64, in order: colour and name.
const PALETTE: [([u8; 3], &str); 64] = [
    ([14, 31, 124], "Prussian Blue"),
    ([10, 85, 163], "Blue"),
    ([0, 135, 119], "Teal Green"),
    ([75, 107, 175], "Cornflower Blue"),
    ([237, 23, 31], "Red"),
    ([209, 92, 0], "Reddish Brown"),
    ([145, 54, 151], "Magenta"),
    ([228, 154, 203], "Light Lilac"),
    ([145, 95, 172], "Lilac"),
    ([158, 214, 125], "Mint Green"),
    ([232, 169, 0], "Deep Gold"),
    ([254, 186, 53], "Orange"),
    ([255, 255, 0], "Yellow"),
    ([112, 188, 31], "Lime Green"),
    ([186, 152, 0], "Brass"),
    ([168, 168, 168], "Silver"),
    ([125, 111, 0], "Russet Brown"),
    ([255, 255, 179], "Cream Brown"),
    ([79, 85, 86], "Pewter"),
    ([0, 0, 0], "Black"),
    ([11, 61, 145], "Ultramarine"),
    ([119, 1, 118], "Royal Purple"),
    ([41, 49, 51], "Dark Gray"),
    ([42, 19, 1], "Dark Brown"),
    ([246, 74, 138], "Deep Rose"),
    ([178, 118, 36], "Light Brown"),
    ([252, 187, 197], "Salmon Pink"),
    ([254, 55, 15], "Vermilion"),
    ([240, 240, 240], "White"),
    ([106, 28, 138], "Violet"),
    ([168, 221, 196], "Seacrest"),
    ([37, 132, 187], "Sky Blue"),
    ([254, 179, 67], "Pumpkin"),
    ([255, 243, 107], "Cream Yellow"),
    ([208, 166, 96], "Khaki"),
    ([209, 84, 0], "Clay Brown"),
    ([102, 186, 73], "Leaf Green"),
    ([19, 74, 70], "Peacock Blue"),
    ([135, 135, 135], "Gray"),
    ([216, 204, 198], "Warm Gray"),
    ([67, 86, 7], "Dark Olive"),
    ([253, 217, 222], "Flesh Pink"),
    ([249, 147, 188], "Pink"),
    ([0, 56, 34], "Deep Green"),
    ([178, 175, 212], "Lavender"),
    ([104, 106, 176], "Wisteria Violet"),
    ([239, 227, 185], "Beige"),
    ([247, 56, 102], "Carmine"),
    ([181, 75, 100], "Amber Red"),
    ([19, 43, 26], "Olive Green"),
    ([199, 1, 86], "Dark Fuchsia"),
    ([254, 158, 50], "Tangerine"),
    ([168, 222, 235], "Light Blue"),
    ([0, 103, 62], "Emerald Green"),
    ([78, 41, 144], "Purple"),
    ([47, 126, 32], "Moss Green"),
    ([255, 204, 204], "Flesh Pink"),
    ([255, 217, 17], "Harvest Gold"),
    ([9, 91, 166], "Electric Blue"),
    ([240, 249, 112], "Lemon Yellow"),
    ([227, 243, 91], "Fresh Green"),
    ([255, 153, 0], "Orange"),
    ([255, 240, 141], "Cream Yellow"),
    ([255, 200, 200], "Applique"),
];

/// The thread of palette entry `index` (1 to 64), if there is one.
pub(super) fn thread(index: u8) -> Option<Thread> {
    let (rgb, name) = PALETTE.get(usize::from(index).checked_sub(1)?)?;
    Some(Thread { rgb: *rgb, name: Some((*name).to_string()), code: Some(index.to_string()), brand: Some("Brother".to_string()) })
}

/// The "redmean" distance between two colours (https://www.compuphase.com/cmetric.htm), which
/// readers of the format use to pick a palette entry.
fn distance(a: [u8; 3], b: [u8; 3]) -> i64 {
    let [r1, g1, b1] = a.map(i64::from);
    let [r2, g2, b2] = b.map(i64::from);
    let mean = (r1 + r2 + 1) / 2;
    let (r, g, b) = (r1 - r2, g1 - g2, b1 - b2);
    (((512 + mean) * r * r) >> 8) + 4 * g * g + (((767 - mean) * b * b) >> 8)
}

/// The palette entry (1 to 64) nearest to `rgb` among those `free` allows; the later of equal
/// distances wins. `None` when none is free.
pub(super) fn nearest(rgb: [u8; 3], free: impl Fn(u8) -> bool) -> Option<u8> {
    let mut best: Option<(i64, u8)> = None;
    for (i, (c, _)) in PALETTE.iter().enumerate() {
        let Ok(index) = u8::try_from(i + 1) else { continue };
        if !free(index) {
            continue;
        }
        let d = distance(rgb, *c);
        if best.is_none_or(|(bd, _)| d <= bd) {
            best = Some((d, index));
        }
    }
    best.map(|(_, i)| i)
}

/// The palette entry for each colour of `colours`: each distinct colour, in order of first use,
/// takes the nearest entry no earlier colour took, so two colours never share an entry while
/// entries remain; past 64 colours, the nearest entry.
pub(super) fn unique_indices(colours: &[[u8; 3]]) -> Vec<u8> {
    let mut taken: Vec<([u8; 3], u8)> = Vec::new();
    colours
        .iter()
        .map(|c| {
            if let Some((_, i)) = taken.iter().find(|(t, _)| t == c) {
                return *i;
            }
            let i = nearest(*c, |i| taken.iter().all(|(_, t)| *t != i)).or_else(|| nearest(*c, |_| true)).unwrap_or(1);
            taken.push((*c, i));
            i
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_and_matching() {
        assert_eq!(thread(20).map(|t| t.rgb), Some([0, 0, 0]));
        assert_eq!(thread(5).and_then(|t| t.name), Some("Red".to_string()));
        assert_eq!(thread(0), None);
        assert_eq!(thread(65), None);
        assert_eq!(nearest([0, 0, 0], |_| true), Some(20));
        assert_eq!(nearest([237, 23, 31], |_| true), Some(5));
        // Two near blacks get different entries; the same colour again keeps its entry.
        let u = unique_indices(&[[0, 0, 0], [1, 1, 1], [0, 0, 0]]);
        assert_eq!(u[0], 20);
        assert_ne!(u[1], 20);
        assert_eq!(u[2], 20);
        let many: Vec<[u8; 3]> = (0..70u8).map(|i| [i, i, i]).collect();
        assert!(unique_indices(&many).iter().all(|i| (1..=64).contains(i)));
    }
}
