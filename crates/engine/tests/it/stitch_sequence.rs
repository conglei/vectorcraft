//! Embroidery sequencing, end to end (STI-19 acceptance): a design of 20 stroked objects in three
//! thread colours, sewn in document order with the colours interleaved and every other object
//! drawn backwards. Sort by Colour brings it to two colour changes; Closest Join then turns the
//! backwards objects so each colour sews as one chain, with fewer trims and less travel.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

const COLOURS: [&str; 3] = ["#d01c1c", "#1c9c3c", "#1c3cd0"];

/// Object `i` is the `j`-th link (j = i / 3) of colour `i % 3`'s zigzag chain, drawn backwards
/// when `j` is odd.
fn design() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    let point = |c: usize, j: usize| (100.0 + 40.0 * j as f64, 100.0 + 150.0 * c as f64 + 40.0 * (j % 2) as f64);
    for i in 0..20 {
        let (c, j) = (i % 3, i / 3);
        s.execute("select.none", &json!({})).unwrap();
        s.execute("paint.setStroke", &json!({"color": COLOURS[c]})).unwrap();
        let (mut a, mut b) = (point(c, j), point(c, j + 1));
        if j % 2 == 1 {
            std::mem::swap(&mut a, &mut b);
        }
        s.execute("shape.line", &json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1})).unwrap();
    }
    s
}

fn info(s: &mut Session) -> Value {
    s.execute("stitch.info", &json!({})).unwrap()
}

#[test]
fn colour_sort_and_closest_join_cut_colour_changes_trims_and_travel() {
    let mut s = design();
    let start = info(&mut s);
    assert_eq!(start["colorChanges"], json!(19), "{start}");
    assert_eq!(s.execute("stitch.sequence.list", &json!({})).unwrap()["objects"].as_array().map(Vec::len), Some(20));

    let r = s.execute("stitch.sequence.sortByColor", &json!({})).unwrap();
    assert_eq!(r["colorChanges"], json!({"before": 19, "after": 2}));
    let sorted = info(&mut s);
    assert_eq!(sorted["colorChanges"], json!(2), "{sorted}");
    assert_eq!(sorted["stitches"], start["stitches"], "the same objects, sewn once each");

    let r = s.execute("stitch.join.closest", &json!({})).unwrap();
    assert_eq!(r["changed"].as_array().map(Vec::len), Some(20), "{r}");
    let joined = info(&mut s);
    assert_eq!(joined["colorChanges"], json!(2), "{joined}");
    // Each colour is now one chain: no trims, and the travel left is mostly between the colours.
    assert_eq!((sorted["trims"].as_u64(), joined["trims"].as_u64()), (Some(17), Some(0)), "{sorted}\n{joined}");
    assert_eq!((sorted["jumps"].as_u64(), joined["jumps"].as_u64()), (Some(33), Some(16)));
    let travel = |v: &Value| v["travelMm"].as_f64();
    assert_eq!((travel(&start), travel(&sorted), travel(&joined)), (Some(1055.1), Some(356.8), Some(188.3)));
    // The model's figures for the same orders: every join and the travel between colours.
    assert_eq!(r["before"], json!({"trims": 17, "travelMm": 551.3, "colorChanges": 2}));
    assert_eq!(r["after"], json!({"trims": 0, "travelMm": 212.3, "colorChanges": 2}));
    // Runs joined without a trim need no lock stitches between them: 300 needle points → 181.
    assert_eq!((sorted["stitches"].as_u64(), joined["stitches"].as_u64()), (Some(300), Some(181)));
    // A second closest join has nothing left to gain.
    assert_eq!(s.execute("stitch.join.closest", &json!({})).unwrap()["changed"], json!([]));

    // One undo step each: back to sorted, then to document order.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(info(&mut s)["trims"], sorted["trims"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(info(&mut s)["colorChanges"], json!(19));
}
