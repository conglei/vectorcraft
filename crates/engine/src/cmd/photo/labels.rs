//! Photo stitch, second pass (STI-30): the photo's colour labels cleaned up before they are traced,
//! so that what one thread sews is one area.
//!
//! The photo is reduced to colours as before (`vectorcraft_trace::quantize`), then every pixel is
//! relabelled by the chart thread its colour matches: neighbouring photo colours that sew in the
//! same thread become one area. Thin slivers of another thread between two areas of the same
//! thread (no wider than `gap` pixels, across a row or a column) are closed, and areas smaller than
//! `min_region` pixels are absorbed into the neighbour they share the longest border with
//! (`vectorcraft_trace::denoise`). Every step is one or a few passes over the pixels.
//! Spec: `docs/superpowers/specs/2026-11-12-embroidery-photo-second-pass.md`.

use vectorcraft_trace::{TRANSPARENT, denoise};

/// The widest gap closed, in pixels.
pub(super) const MAX_GAP_PX: usize = 32;

/// What the cleanup did, in areas (4-connected regions of one label).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Cleanup {
    /// The areas of the photo's colours before cleanup (what the first pass sewed one by one).
    pub before: usize,
    /// Areas that became part of another area of the same thread: by relabelling or a closed gap.
    pub merged: usize,
    /// Areas absorbed into a neighbour of another thread because they were too small.
    pub absorbed: usize,
    /// Areas left.
    pub after: usize,
}

/// How many 4-connected areas of one label `labels` (`w × h`) has; transparent pixels aren't any.
pub(super) fn areas(labels: &[u16], w: usize, h: usize) -> usize {
    if w.checked_mul(h) != Some(labels.len()) {
        return 0;
    }
    let mut seen = vec![false; labels.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut n = 0;
    for start in 0..labels.len() {
        let Some(&l) = labels.get(start) else { continue };
        if l == TRANSPARENT || seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        n += 1;
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
                if labels.get(j) == Some(&l) && seen.get(j) == Some(&false) {
                    if let Some(s) = seen.get_mut(j) {
                        *s = true;
                    }
                    stack.push(j);
                }
            }
        }
    }
    n
}

/// Close the runs of a label no longer than `gap` pixels that have the same other label at both
/// ends, along rows and then along columns. Transparent pixels are never filled and never close
/// a gap. → pixels changed.
pub(super) fn close_gaps(labels: &mut [u16], w: usize, h: usize, gap: usize) -> usize {
    let gap = gap.min(MAX_GAP_PX);
    if gap == 0 || w.checked_mul(h) != Some(labels.len()) {
        return 0;
    }
    let mut changed = 0;
    // A line of pixels: its first index and the step between them, and its length.
    let rows = (0..h).map(|y| (y * w, 1, w));
    let cols = (0..w).map(|x| (x, w, h));
    for (start, step, len) in rows.chain(cols).collect::<Vec<_>>() {
        let at = |k: usize| start + k * step;
        let mut k = 0;
        while k < len {
            let Some(&l) = labels.get(at(k)) else { break };
            let mut end = k + 1;
            while end < len && labels.get(at(end)) == Some(&l) {
                end += 1;
            }
            // The run is k..end: closed when it is short and the same label lies on both sides.
            if l != TRANSPARENT && k > 0 && end < len && end - k <= gap {
                let (before, after) = (labels.get(at(k - 1)).copied(), labels.get(at(end)).copied());
                if let (Some(b), Some(a)) = (before, after)
                    && a == b
                    && a != TRANSPARENT
                {
                    for q in k..end {
                        if let Some(p) = labels.get_mut(at(q)) {
                            *p = a;
                            changed += 1;
                        }
                    }
                }
            }
            k = end;
        }
    }
    changed
}

/// `photo` (photo colour labels, `w × h`) relabelled by `thread_of` each colour, gaps up to `gap`
/// pixels closed and areas under `min_region` pixels absorbed. → (thread labels, what was done)
pub(super) fn by_thread(photo: &[u16], w: usize, h: usize, thread_of: &[u16], gap: usize, min_region: usize) -> (Vec<u16>, Cleanup) {
    let before = areas(photo, w, h);
    let mut labels: Vec<u16> =
        photo.iter().map(|&l| if l == TRANSPARENT { l } else { thread_of.get(l as usize).copied().unwrap_or(TRANSPARENT) }).collect();
    close_gaps(&mut labels, w, h, gap);
    let joined = areas(&labels, w, h);
    denoise(&mut labels, w, h, min_region);
    let after = areas(&labels, w, h);
    (labels, Cleanup { before, merged: before.saturating_sub(joined), absorbed: joined.saturating_sub(after), after })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_thread_colours_become_one_area_and_slivers_close() {
        // Two photo colours (0, 1) of thread 0 side by side, a one-pixel column of colour 2 (thread
        // 1) through them, and a 2 × 2 speck of colour 2 in the corner.
        let (w, h) = (12, 6);
        let mut photo = vec![0u16; w * h];
        for y in 0..h {
            for x in 6..w {
                photo[y * w + x] = 1;
            }
            photo[y * w + 3] = 2;
        }
        for (x, y) in [(10, 4), (11, 4), (10, 5), (11, 5)] {
            photo[y * w + x] = 2;
        }
        assert_eq!(areas(&photo, w, h), 5);
        let (labels, c) = by_thread(&photo, w, h, &[0, 0, 1], 1, 5);
        assert!(labels.iter().all(|&l| l == 0), "{labels:?}");
        assert_eq!(c, Cleanup { before: 5, merged: 3, absorbed: 1, after: 1 });
        // Without closing, the column stays (it is larger than the smallest area kept).
        let (labels, c) = by_thread(&photo, w, h, &[0, 0, 1], 0, 5);
        assert_eq!(labels.iter().filter(|&&l| l == 1).count(), h);
        assert_eq!(c.after, 3);
    }

    #[test]
    fn transparency_is_never_filled_or_bridged() {
        let (w, h) = (5, 1);
        let mut labels = vec![0, TRANSPARENT, 0, 1, 0];
        assert_eq!(close_gaps(&mut labels, w, h, 2), 1);
        assert_eq!(labels, vec![0, TRANSPARENT, 0, 0, 0]);
        assert_eq!(close_gaps(&mut [0u16; 3], 2, 2, 1), 0, "the wrong size is left alone");
        assert_eq!(areas(&[TRANSPARENT; 4], 2, 2), 0);
    }
}
