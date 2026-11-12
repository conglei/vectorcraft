//! Acceptance for photo stitch (gap E11): the synthetic fur picture through `photo.toStitches` to
//! a DST under 100 000 stitches, the stitches of each half running within 15° of the hair drawn
//! there, and `stitch.preview` rendering it (`--nocapture` prints where the preview was written).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_imageprep::{FUR_ANGLES, encode_png, synthetic_fur, synthetic_fur_angle};

const PT_PER_MM: f64 = 72.0 / 25.4;

/// Every path under `n`, depth first.
fn paths<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    if matches!(n.kind, NodeKind::Path { .. }) {
        out.push(n);
    }
    for c in n.children().into_iter().flatten() {
        paths(c, out);
    }
}

fn angle_error(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(180.0);
    d.min(180.0 - d)
}

#[test]
fn the_synthetic_fur_becomes_a_dst_whose_rows_follow_the_hair() {
    let (w, h) = (360u32, 270u32);
    let width_mm = 90.0;
    let dir = std::env::temp_dir().join(format!("vc-photo-stitch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let photo = dir.join("fur.png");
    std::fs::write(&photo, encode_png(&synthetic_fur(w, h, 5)).unwrap()).unwrap();

    let mut s = Session::new();
    let r = s.execute("photo.toStitches", &json!({"file": photo, "provider": "none", "widthMm": width_mm, "feather": 1.0, "maxSide": 360})).unwrap();
    let stitches = r["stitches"].as_u64().unwrap();
    assert!(stitches > 5_000 && stitches < 100_000, "{r}");

    let dst = dir.join("fur.dst");
    s.execute("document.export", &json!({"path": dst, "format": "dst"})).unwrap();
    let bytes = std::fs::read(&dst).unwrap();
    assert!(bytes.starts_with(b"LA:"), "a DST header");
    let info = s.execute("stitch.info", &json!({})).unwrap();
    assert_eq!(info["stitches"].as_u64(), Some(stitches));

    // The needle points: the paths of the stitch layer, in mm. Per half, the length-weighted mean
    // direction of the stitches (lock stitches and travel shorter than 1 mm left out).
    let doc = &s.doc().unwrap().doc;
    let layer = doc.layers.iter().find(|l| l.name.as_deref() == Some("Stitches")).unwrap();
    let mut found = vec![];
    paths(layer, &mut found);
    let mm_per_px = width_mm / f64::from(w);
    let mut sums = [(0.0f64, 0.0f64, 0.0f64); 2];
    for p in found {
        let NodeKind::Path { path, .. } = &p.kind else { continue };
        for sp in &path.subpaths {
            for pair in sp.anchors.windows(2) {
                let (a, b) = ((pair[0].p.x / PT_PER_MM, pair[0].p.y / PT_PER_MM), (pair[1].p.x / PT_PER_MM, pair[1].p.y / PT_PER_MM));
                let len = (b.0 - a.0).hypot(b.1 - a.1);
                if len < 1.0 {
                    continue;
                }
                let mid = ((a.0 + b.0) / 2.0 / mm_per_px, (a.1 + b.1) / 2.0 / mm_per_px);
                let Some(want) = synthetic_fur_angle(w, h, mid.0, mid.1, 12.0) else { continue };
                let half = usize::from(want == FUR_ANGLES.1);
                let t = 2.0 * (-(b.1 - a.1)).atan2(b.0 - a.0);
                sums[half].0 += len * t.cos();
                sums[half].1 += len * t.sin();
                sums[half].2 += len;
            }
        }
    }
    for (half, want) in [(0, FUR_ANGLES.0), (1, FUR_ANGLES.1)] {
        let (x, y, total) = sums[half];
        assert!(total > 1_000.0, "half {half}: only {total} mm of stitches");
        let got = (y.atan2(x).to_degrees() / 2.0).rem_euclid(180.0);
        let err = angle_error(got, want);
        // How strongly the stitches agree: 1 when all run one way.
        let agreement = x.hypot(y) / total;
        println!("half {half}: dominant direction {got:.1}° (want {want}°, off {err:.1}°), agreement {agreement:.2}, {total:.0} mm");
        assert!(err < 15.0, "half {half}: {got}° is {err}° off {want}°");
    }

    let preview = dir.join("preview.png");
    let p = s.execute("stitch.preview", &json!({"path": preview, "scale": 4})).unwrap();
    assert!(p["width"].as_u64().unwrap() > 300, "{p}");
    let png = std::fs::read(&preview).unwrap();
    assert_eq!(&png[1..4], b"PNG");
    println!("{stitches} stitches; preview: {}", preview.display());
}
