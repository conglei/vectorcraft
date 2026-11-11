//! Thread colours for files that store none: a small original list, cycled.

use vectorcraft_stitch::Thread;

const COLOURS: [[u8; 3]; 12] = [
    [0, 0, 0],
    [200, 30, 40],
    [30, 90, 200],
    [20, 140, 70],
    [240, 200, 20],
    [240, 120, 20],
    [130, 50, 160],
    [120, 70, 30],
    [240, 130, 170],
    [40, 170, 190],
    [120, 120, 120],
    [250, 250, 250],
];

/// The thread for block `index` of a file that stores no colours.
pub fn default_thread(index: usize) -> Thread {
    let rgb = COLOURS.get(index % COLOURS.len()).copied().unwrap_or_default();
    Thread { rgb, name: None, code: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_cycles() {
        assert_eq!(default_thread(0).rgb, [0, 0, 0]);
        assert_eq!(default_thread(11).rgb, [250, 250, 250]);
        assert_eq!(default_thread(13), default_thread(1));
    }
}
