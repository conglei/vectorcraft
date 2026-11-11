//! Embroidery thread charts: import, listing, the active chart, matching, needles and `stitch.info`.

use serde_json::{Value, json};

use crate::*;

const CHART: &str =
    "brand,code,name,hex,weight\nAcme,100,Cherry,#c81e28,40wt\nAcme,200,Ocean,#1e50c8,40wt\nAcme,300,Snow,#fafafa,\nAcme,400,Coal,#101010,\n";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A red and a blue stroked rectangle: two thread colours.
fn two_colours(s: &mut Session) {
    run(s, "paint.setFill", json!({"none": true}));
    for (color, x) in [("#ff0000", 0), ("#0000ff", 120)] {
        run(s, "select.none", json!({}));
        run(s, "paint.setStroke", json!({ "color": color }));
        run(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 50}));
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-threads-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn import_acme(s: &mut Session) -> Value {
    run(s, "stitch.threads.import", json!({"data": CHART, "name": "Acme 40.csv"}))
}

#[test]
fn the_list_has_the_starter_set_and_an_imported_chart() {
    let mut s = session();
    let r = run(&mut s, "stitch.threads.list", json!({}));
    let charts = r["charts"].as_array().unwrap();
    assert_eq!(charts.len(), 1);
    assert_eq!(
        (charts[0]["id"].as_str(), charts[0]["count"].as_u64(), charts[0]["category"].as_str()),
        (Some("builtin/starter"), Some(24), Some("builtIn"))
    );
    assert_eq!(r["active"], Value::Null);
    let i = import_acme(&mut s);
    assert_eq!(
        (i["name"].as_str(), i["count"].as_u64(), i["category"].as_str(), i["copied"].as_bool()),
        (Some("Acme 40"), Some(4), Some("loaded"), Some(false))
    );
    let r = run(&mut s, "stitch.threads.list", json!({"chart": "acme 40"}));
    assert_eq!(r["charts"].as_array().unwrap().len(), 2);
    let t = r["threads"].as_array().unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t[0], json!({"brand": "Acme", "code": "100", "name": "Cherry", "hex": "#c81e28", "weight": "40wt"}));
    assert!(s.execute("stitch.threads.list", &json!({"chart": "nope"})).is_err());
}

#[test]
fn bad_charts_are_errors_with_the_line() {
    let mut s = session();
    let e = s.execute("stitch.threads.import", &json!({"data": "code,hex\n1,#112233\n2,blue"})).unwrap_err().to_string();
    assert!(e.contains("line 3") && e.contains("not a colour"), "{e}");
    for p in [
        json!({}),
        json!({"data": ""}),
        json!({"data": "{}"}),
        json!({"dataBase64": "!!"}),
        json!({"path": "/nonexistent/chart.csv"}),
        json!({"data": CHART, "user": true}),
    ] {
        assert!(s.execute("stitch.threads.import", &p).is_err(), "{p}");
    }
    assert_eq!(run(&mut s, "stitch.threads.list", json!({}))["charts"].as_array().unwrap().len(), 1);
}

#[test]
fn a_chart_is_read_from_a_path_and_kept_in_the_threads_folder() {
    let dir = temp_dir("user");
    let src = temp_dir("src");
    std::fs::create_dir_all(&src).unwrap();
    let file = src.join("Acme Chart.csv");
    std::fs::write(&file, CHART).unwrap();
    let mut s = session();
    s.thread_charts.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "stitch.threads.import", json!({"path": file.to_string_lossy(), "user": true}));
    assert_eq!((r["category"].as_str(), r["copied"].as_bool(), r["count"].as_u64()), (Some("user"), Some(true), Some(4)), "{r}");
    assert!(dir.join("Acme Chart.csv").is_file());
    // A new session on the same folder lists it.
    let mut s2 = session();
    s2.thread_charts.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let list = run(&mut s2, "stitch.threads.list", json!({}));
    let user: Vec<&Value> = list["charts"].as_array().unwrap().iter().filter(|c| c["category"] == "user").collect();
    assert_eq!((user.len(), user[0]["name"].as_str(), user[0]["count"].as_u64()), (1, Some("Acme Chart"), Some(4)));
    assert_eq!(list["userFolder"], json!(dir.to_string_lossy()));
    // JSON data is kept as a JSON file under the chart's name.
    let j = run(&mut s2, "stitch.threads.import", json!({"data": starter_json(), "user": true}));
    assert_eq!((j["copied"].as_bool(), j["name"].as_str()), (Some(true), Some("VectorCraft Starter")), "{j}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&src);
}

fn starter_json() -> String {
    vectorcraft_stitch::starter_chart().to_json()
}

#[test]
fn a_chart_past_the_size_cap_is_refused_before_it_is_read_in_full() {
    let dir = temp_dir("big");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("big.csv");
    std::fs::write(&file, vec![b'a'; vectorcraft_stitch::MAX_CHART_BYTES + 1]).unwrap();
    let e = session().execute("stitch.threads.import", &json!({"path": file.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("larger than"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn use_makes_a_chart_active_and_stitch_info_names_the_nearest_threads() {
    let mut s = session();
    two_colours(&mut s);
    let plain = run(&mut s, "stitch.info", json!({}));
    assert!(plain["threads"][0].get("brand").is_none());
    import_acme(&mut s);
    let u = run(&mut s, "stitch.threads.use", json!({"name": "ACME 40"}));
    assert_eq!((u["chart"].as_str(), u["count"].as_u64()), (Some("loaded/Acme 40.csv"), Some(4)));
    assert_eq!(run(&mut s, "stitch.threads.list", json!({}))["active"], "loaded/Acme 40.csv");
    let info = run(&mut s, "stitch.info", json!({}));
    let t = info["threads"].as_array().unwrap();
    assert_eq!(t.len(), 2);
    assert_eq!((t[0]["brand"].as_str(), t[0]["code"].as_str(), t[0]["name"].as_str()), (Some("Acme"), Some("100"), Some("Cherry")));
    assert_eq!((t[1]["code"].as_str(), t[1]["name"].as_str()), (Some("200"), Some("Ocean")));
    assert!(t[0]["deltaE"].as_f64().unwrap() > 0.0);
    // The rest of the report is unchanged.
    assert_eq!(info["stitches"], plain["stitches"]);
    // Clearing it, or a chart that is gone, drops the names.
    run(&mut s, "stitch.threads.use", json!({"name": "none"}));
    assert!(run(&mut s, "stitch.info", json!({}))["threads"][0].get("code").is_none());
    assert!(s.execute("stitch.threads.use", &json!({"name": "missing"})).is_err());
    let r = run(&mut s, "stitch.threads.use", json!({}));
    assert_eq!(r["chart"], Value::Null);
}

#[test]
fn match_gives_one_row_per_block_with_the_threads_and_the_distance() {
    let mut s = session();
    two_colours(&mut s);
    assert!(s.execute("stitch.threads.match", &json!({})).unwrap_err().to_string().contains("no thread chart is active"));
    let r = run(&mut s, "stitch.threads.match", json!({"chart": "VectorCraft Starter"}));
    let b = r["blocks"].as_array().unwrap();
    assert_eq!(b.len(), 2);
    assert_eq!(
        (b[0]["block"].as_u64(), b[0]["rgb"].as_str(), b[0]["brand"].as_str(), b[0]["name"].as_str()),
        (Some(0), Some("#ff0000"), Some("VectorCraft"), Some("Red"))
    );
    assert_eq!(b[1]["name"], "Blue");
    import_acme(&mut s);
    run(&mut s, "stitch.threads.use", json!({"name": "Acme 40"}));
    let r = run(&mut s, "stitch.threads.match", json!({}));
    assert_eq!(r["blocks"][0]["code"], "100");
    assert_eq!(r["blocks"][0]["thread"], "#c81e28");
    assert!(s.execute("stitch.threads.match", &json!({"chart": "nope"})).is_err());
    assert!(session().execute("stitch.threads.match", &json!({"chart": "VectorCraft Starter"})).is_err());
}

#[test]
fn needles_follow_the_sewing_order_and_name_the_chart_thread() {
    let mut s = session();
    two_colours(&mut s);
    // A third rectangle in the first colour again: it keeps needle 1.
    run(&mut s, "select.none", json!({}));
    run(&mut s, "paint.setStroke", json!({"color": "#ff0000"}));
    run(&mut s, "shape.rectangle", json!({"x": 240, "y": 0, "width": 50, "height": 50}));
    let r = run(&mut s, "stitch.needles", json!({}));
    assert_eq!((r["machine"].as_str(), r["needleCount"].as_u64()), (Some("commercial15"), Some(15)));
    let n = r["needles"].as_array().unwrap();
    assert_eq!(n.len(), 2, "{r}");
    assert_eq!((n[0]["needle"].as_u64(), n[0]["rgb"].as_str()), (Some(1), Some("#ff0000")));
    assert_eq!((n[1]["needle"].as_u64(), n[1]["rgb"].as_str()), (Some(2), Some("#0000ff")));
    assert!(n[0]["stitches"].as_u64().unwrap() > 0 && r["warnings"].as_array().unwrap().is_empty());
    assert!(n[0].get("chart").is_none());
    let r = run(&mut s, "stitch.needles", json!({"machine": "bai_mirror", "chart": "VectorCraft Starter"}));
    assert_eq!(r["needles"][0]["chart"]["name"], "Red");
    assert_eq!(r["needles"][1]["chart"]["code"].as_str().map(str::len), Some(4));
    assert!(s.execute("stitch.needles", &json!({"machine": "x"})).is_err());
    assert!(s.execute("stitch.needles", &json!({"chart": "nope"})).is_err());
    assert!(session().execute("stitch.needles", &json!({})).is_err());
}

#[test]
fn more_colours_than_needles_warn() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"none": true}));
    for i in 0..16u32 {
        run(&mut s, "select.none", json!({}));
        run(&mut s, "paint.setStroke", json!({"color": format!("#{:02x}0000", 20 + i * 12)}));
        run(&mut s, "shape.rectangle", json!({"x": i * 12, "y": 0, "width": 10, "height": 10}));
    }
    let r = run(&mut s, "stitch.needles", json!({}));
    let n = r["needles"].as_array().unwrap();
    assert_eq!(n.len(), 16);
    assert_eq!((n[14]["needle"].as_u64(), n[15]["needle"].clone()), (Some(15), Value::Null));
    let w = r["warnings"][0].as_str().unwrap();
    assert!(w.contains("16 thread colours") && w.contains("15 needles"), "{w}");
    let free = run(&mut s, "stitch.needles", json!({"machine": "generic"}));
    assert_eq!(
        (free["needleCount"].clone(), free["needles"][15]["needle"].as_u64(), free["warnings"].as_array().map(Vec::len)),
        (Value::Null, Some(16), Some(0))
    );
}
