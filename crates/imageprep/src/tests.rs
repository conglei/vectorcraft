use super::*;

fn blank(w: u32, h: u32, rgba: [u8; 4]) -> Raster {
    Raster::from_fn(w, h, |_, _| rgba)
}

#[test]
fn no_model_scales_down_cleans_up_and_says_it_kept_the_background() {
    let photo = synthetic_photo(640, 480, 1);
    let out = NoModel.prepare(&photo, &PrepareOptions { max_side: 320, ..Default::default() }).unwrap();
    assert_eq!((out.image.width, out.image.height), (320, 240));
    assert_eq!(out.provider, NO_MODEL);
    assert!(out.notes.iter().any(|n| n.contains("background was not removed")), "{:?}", out.notes);
    assert!(out.notes.iter().any(|n| n.contains("smoothed")), "{:?}", out.notes);
    let l = likeness(&photo, &out.image);
    assert!(l.score >= LIKENESS_THRESHOLD, "{l:?}");
    // The clean-up lowers the background's noise.
    let spread = |r: &Raster| {
        let v: Vec<u8> = (0..20).flat_map(|y| (0..20).map(move |x| (x, y))).map(|(x, y)| r.pixel(x, y)[0]).collect();
        v.iter().max().unwrap() - v.iter().min().unwrap()
    };
    let small = downscale(&photo, 320).unwrap();
    assert!(spread(&out.image) < spread(&small), "{} {}", spread(&out.image), spread(&small));
}

#[test]
fn no_model_keeps_a_small_photo_and_flattens_alpha_when_asked() {
    let photo = Raster::from_fn(100, 80, |x, _| if x < 50 { [200, 10, 10, 0] } else { [10, 10, 200, 255] });
    let o = PrepareOptions { keep_alpha: false, clean_up: false, ..Default::default() };
    let out = NoModel.prepare(&photo, &o).unwrap();
    assert_eq!((out.image.width, out.image.height), (100, 80));
    assert_eq!(out.image.pixel(10, 10), [255, 255, 255, 255]);
    assert_eq!(out.image.pixel(90, 10), [10, 10, 200, 255]);
    let kept = NoModel.prepare(&photo, &PrepareOptions { clean_up: false, ..Default::default() }).unwrap();
    assert_eq!(kept.image, photo);
}

#[test]
fn bad_rasters_are_errors_not_crashes() {
    let broken = Raster { width: 10, height: 10, rgba: vec![0; 7] };
    assert!(NoModel.prepare(&broken, &PrepareOptions::default()).is_err());
    let empty = Raster { width: 0, height: 0, rgba: vec![] };
    assert!(NoModel.prepare(&empty, &PrepareOptions::default()).is_err());
    assert_eq!(likeness(&broken, &broken).score, 0.0);
    assert!(resize_exact(&synthetic_photo(10, 10, 0), 0, 5).is_err());
    assert!(resize_exact(&synthetic_photo(10, 10, 0), 5000, 5).is_err());
    assert!(decode_capped(b"not an image", 1 << 20).is_err());
}

#[test]
fn fit_side_keeps_the_aspect() {
    assert_eq!(fit_side(4000, 3000, 1024), (1024, 768));
    assert_eq!(fit_side(3000, 4000, 1024), (768, 1024));
    assert_eq!(fit_side(800, 600, 1024), (800, 600));
    assert_eq!(fit_side(100_000, 1, 1024), (1024, 1));
}

#[test]
fn decode_capped_refuses_large_images_before_decoding() {
    let png = encode_png(&synthetic_photo(300, 200, 2)).unwrap();
    let back = decode_capped(&png, 1 << 20).unwrap();
    assert_eq!((back.width, back.height), (300, 200));
    let e = decode_capped(&png, 50_000).unwrap_err();
    assert!(e.contains("300 × 200"), "{e}");
}

#[test]
fn identical_images_score_one() {
    let photo = synthetic_photo(400, 300, 3);
    let l = likeness(&photo, &photo);
    assert!(l.score > 0.999, "{l:?}");
    // The noisy background averages out to a flat colour at the comparison size: left out.
    assert!(l.subject > 0.1, "{l:?}");
}

#[test]
fn a_blank_image_scores_low() {
    let photo = synthetic_photo(400, 300, 4);
    for b in [blank(400, 300, [255, 255, 255, 255]), blank(400, 300, [0, 0, 0, 0]), blank(400, 300, [128, 128, 128, 255])] {
        let l = likeness(&photo, &b);
        assert!(l.score < 0.1, "{l:?}");
        assert!(l.notes.iter().any(|n| n.contains("no subject")), "{l:?}");
    }
}

#[test]
fn a_different_image_scores_below_the_threshold() {
    let photo = synthetic_photo(400, 300, 5);
    // Stripes where the photo has a blob.
    let other = Raster::from_fn(400, 300, |x, y| if (x / 12 + y / 12) % 2 == 0 { [20, 200, 40, 255] } else { [240, 20, 200, 255] });
    let l = likeness(&photo, &other);
    assert!(l.score < LIKENESS_THRESHOLD, "{l:?}");
}

#[test]
fn a_cut_out_subject_is_compared_on_the_subject() {
    let photo = synthetic_photo(400, 300, 6);
    let (cx, cy, r) = (200.0, 165.0, 90.0);
    let inside = |x: u32, y: u32| {
        let (dx, dy) = ((f64::from(x) - cx) / r, (f64::from(y) - cy) / (r * 0.85));
        dx * dx + dy * dy <= 0.95
    };
    // Transparent background, as a model that returns alpha would.
    let cut = Raster::from_fn(400, 300, |x, y| if inside(x, y) { photo.pixel(x, y) } else { [0, 0, 0, 0] });
    let l = likeness(&photo, &cut);
    assert!(l.score > 0.85, "{l:?}");
    assert!(l.subject < 0.5, "{l:?}");
    // A flat white background, as one that can't.
    let flat = Raster::from_fn(400, 300, |x, y| if inside(x, y) { photo.pixel(x, y) } else { [255, 255, 255, 255] });
    let l = likeness(&photo, &flat);
    assert!(l.score > 0.85, "{l:?}");
    assert!(l.notes.iter().any(|n| n.contains("flat background")), "{l:?}");
    // A restyle (colours inverted inside the subject) scores low.
    let restyled = Raster::from_fn(400, 300, |x, y| {
        if inside(x, y) {
            let p = photo.pixel(x, y);
            [255 - p[0], 255 - p[1], 255 - p[2], 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    assert!(likeness(&photo, &restyled).score < LIKENESS_THRESHOLD);
}

#[test]
fn a_reshaped_result_is_noted() {
    let photo = synthetic_photo(400, 300, 7);
    let square = resize_exact(&photo, 300, 300).unwrap();
    let l = likeness(&photo, &square);
    assert!(l.notes.iter().any(|n| n.contains("not the photo's shape")), "{l:?}");
}

#[test]
fn the_synthetic_fur_runs_the_way_it_says() {
    let (w, h) = (240, 180);
    let fur = synthetic_fur(w, h, 3);
    assert_eq!(fur, synthetic_fur(w, h, 3));
    assert_ne!(fur, synthetic_fur(w, h, 4));
    let (eye, _) = synthetic_fur_eye(w, h);
    assert_eq!(fur.pixel(eye.0 as u32, eye.1 as u32), [235, 228, 210, 255]);
    let field = vectorcraft_trace::direction_field(&fur, &vectorcraft_trace::FieldParams::default()).unwrap();
    let mut checked = 0;
    for r in 0..field.rows {
        for c in 0..field.cols {
            let (x, y) = ((c as f64 + 0.5) * field.cell, (r as f64 + 0.5) * field.cell);
            let Some(want) = synthetic_fur_angle(w, h, x, y, 24.0) else { continue };
            if x < 12.0 || y < 12.0 || x > f64::from(w) - 12.0 || y > f64::from(h) - 12.0 {
                continue;
            }
            let (got, coh) = field.cell_at(c, r).unwrap();
            let d = (got - want).rem_euclid(180.0);
            assert!(d.min(180.0 - d) < 10.0, "cell {c},{r}: {got} not {want}");
            assert!(coh > 0.3, "cell {c},{r}: coherence {coh}");
            checked += 1;
        }
    }
    assert!(checked > 150, "{checked}");
    assert_eq!(synthetic_fur_angle(w, h, 30.0, 30.0, 4.0), Some(FUR_ANGLES.0));
    assert_eq!(synthetic_fur_angle(w, h, 200.0, 150.0, 4.0), Some(FUR_ANGLES.1));
    assert_eq!(synthetic_fur_angle(w, h, 121.0, 30.0, 4.0), None);
}
