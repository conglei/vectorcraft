//! The embroidery tools: `stitch_info` and `export_stitch` forward to `stitch.info` and
//! `document.export`, and the `stitch-out` prompt names them.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool, tool_definitions};

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("vectorcraft-mcp-stitch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().to_string()
}

fn text(r: &crate::ToolResult) -> Value {
    assert!(!r.is_error, "{r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

/// A headless session with a 100 × 50 pt rectangle stroked red, without a fill.
fn red_rectangle() -> Headless {
    let mut h = Headless::new();
    let run = |h: &mut Headless, command: &str, params: Value| h.call("engine.execute", json!({"command": command, "params": params})).unwrap();
    run(&mut h, "file.new", json!({"width": 600, "height": 400}));
    run(&mut h, "paint.setFill", json!({"none": true}));
    run(&mut h, "paint.setStroke", json!({"color": "#ff0000"}));
    run(&mut h, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50}));
    h
}

#[test]
fn the_stitch_tools_are_listed_with_the_export_options() {
    let tools = tool_definitions();
    for name in ["stitch_info", "export_stitch"] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("missing tool {name}"));
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        for key in [
            "format",
            "machine",
            "mode",
            "stitchLength",
            "minStitch",
            "maxStitch",
            "locks",
            "jumpsPerTrim",
            "trimBeforeColorChange",
            "start",
            "returnToStart",
            "name",
            "selectedOnly",
        ] {
            assert!(props.contains_key(key), "{name}: {key}");
        }
        assert_eq!(props["format"]["enum"], json!(["dst", "dsb", "exp", "u01", "tbf", "pes", "pec"]), "{name}");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }
    let info = tools.iter().find(|t| t["name"] == "stitch_info").unwrap();
    let export = tools.iter().find(|t| t["name"] == "export_stitch").unwrap();
    assert_eq!(info["annotations"]["readOnlyHint"], true);
    assert_eq!(export["annotations"]["readOnlyHint"], false, "it writes a file");
    assert!(export["inputSchema"]["properties"].get("path").is_some() && info["inputSchema"]["properties"].get("path").is_none());
}

#[test]
fn stitch_info_returns_what_the_command_returns() {
    let mut h = red_rectangle();
    let direct = h.call("engine.execute", json!({"command": "stitch.info", "params": {"stitchLength": 3, "locks": false}})).unwrap();
    let r = text(&call_tool(&mut h, "stitch_info", &json!({"stitchLength": 3, "locks": false})));
    assert_eq!(r, direct);
    assert!(r["stitches"].as_u64().unwrap() > 10, "{r}");
    assert_eq!(r["threads"][0]["rgb"], "#ff0000");
    // The options are the export's: a shorter stitch length makes more stitches, EXP differs from DST.
    let finer = text(&call_tool(&mut h, "stitch_info", &json!({"stitchLength": 1, "locks": false})));
    assert!(finer["stitches"].as_u64() > r["stitches"].as_u64(), "{finer} {r}");
    let exp = text(&call_tool(&mut h, "stitch_info", &json!({"format": "exp", "locks": false, "stitchLength": 3})));
    assert_ne!(exp["records"], r["records"]);
}

#[test]
fn stitch_info_errors_come_back_as_tool_errors() {
    let mut h = red_rectangle();
    let r = call_tool(&mut h, "stitch_info", &json!({"stitchLength": 99}));
    assert!(r.is_error && r.content[0]["text"].as_str().unwrap().contains("stitchLength"), "{r:?}");
    // Nothing stitchable.
    let mut empty = Headless::new();
    empty.call("engine.execute", json!({"command": "file.new", "params": {"width": 100, "height": 100}})).unwrap();
    assert!(call_tool(&mut empty, "stitch_info", &json!({})).is_error);
}

#[test]
fn export_stitch_writes_the_file_the_export_command_writes() {
    let mut h = red_rectangle();
    let dst = tmp("a.dst");
    let r = text(&call_tool(&mut h, "export_stitch", &json!({"path": dst, "locks": false, "name": "RECT"})));
    assert_eq!((r["format"].as_str(), r["path"].as_str()), (Some("dst"), Some(dst.as_str())), "{r}");
    let bytes = std::fs::read(&dst).unwrap();
    assert!(bytes.starts_with(b"LA:RECT"), "{:?}", &bytes[..bytes.len().min(12)]);
    // The extension names the format when none is given; an explicit format wins.
    let exp = tmp("a.exp");
    assert_eq!(text(&call_tool(&mut h, "export_stitch", &json!({"path": exp})))["format"], "exp");
    let named = tmp("b.dat");
    assert_eq!(text(&call_tool(&mut h, "export_stitch", &json!({"path": named, "format": "exp"})))["format"], "exp");
    // The bytes are those of document.export with the same params.
    let direct =
        h.call("engine.execute", json!({"command": "document.export", "params": {"format": "dst", "locks": false, "name": "RECT"}})).unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), vectorcraft_format::base64_decode(direct["dataBase64"].as_str().unwrap()).unwrap());
    // Without a path the bytes come back, in DST.
    let r = text(&call_tool(&mut h, "export_stitch", &json!({})));
    assert_eq!(r["format"], "dst");
    assert!(r["dataBase64"].as_str().is_some_and(|d| !d.is_empty()), "{r}");
    // Bad options are an error, and write nothing.
    let bad = tmp("bad.dst");
    assert!(call_tool(&mut h, "export_stitch", &json!({"path": bad, "jumpsPerTrim": 9})).is_error);
    assert!(!std::path::Path::new(&bad).exists());
}

#[test]
fn the_production_tools_write_the_worksheet_and_the_folder() {
    let tools = tool_definitions();
    for name in ["stitch_worksheet", "export_production"] {
        let tool = tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("missing tool {name}"));
        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{name} writes files");
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        assert!(props.contains_key("machine") && props.contains_key("spm") && !props.contains_key("format"), "{name}");
    }
    let mut h = red_rectangle();
    let sheet = tmp("sheet.pdf");
    let r = text(&call_tool(&mut h, "stitch_worksheet", &json!({"path": sheet, "machine": "bai_mirror"})));
    assert_eq!(r["stops"], 1, "{r}");
    assert!(std::fs::read(&sheet).unwrap().starts_with(b"%PDF"));
    let dir = tmp("production");
    let _ = std::fs::remove_dir_all(&dir);
    let r = text(&call_tool(&mut h, "export_production", &json!({"dir": dir, "machine": "bai_mirror", "name": "rect"})));
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["format"].as_str().unwrap()).collect();
    assert_eq!(files, ["dst", "dsb", "pdf"], "{r}");
    assert!(std::path::Path::new(&dir).join("rect.dsb").is_file());
    assert!(r["findings"].as_array().is_some_and(|f| !f.is_empty()));
    assert!(call_tool(&mut h, "export_production", &json!({"dir": dir, "name": "../x"})).is_error);
    assert!(call_tool(&mut h, "stitch_worksheet", &json!({})).is_error);
}

#[test]
fn the_stitch_out_prompt_names_the_tools() {
    let v = crate::prompts::get("stitch-out", &json!({"design": "a star", "directory": "/tmp/out", "format": "exp"})).unwrap();
    let text = v["messages"][0]["content"]["text"].as_str().unwrap_or_default();
    for step in ["a star", "/tmp/out", "stitch_info", "export_stitch", "stitch.calibration", "draw_path"] {
        assert!(text.contains(step), "{step}: {text}");
    }
    let at = |s: &str| text.find(s).unwrap();
    assert!(at("stitch_info with") < at("export_stitch with"), "{text}");
    assert!(crate::prompts::get("stitch-out", &json!({})).is_err(), "the design is required");
}

#[test]
fn photo_to_stitches_runs_the_command_and_writes_the_file() {
    let tools = tool_definitions();
    let tool = tools.iter().find(|t| t["name"] == "photo_to_stitches").unwrap();
    assert_eq!(tool["annotations"]["readOnlyHint"], false, "it opens a document and writes a file");
    let props = tool["inputSchema"]["properties"].as_object().unwrap();
    for key in ["file", "image", "provider", "colors", "widthMm", "fill", "spacing", "minLength", "maxLength", "feather", "seed", "path", "format"] {
        assert!(props.contains_key(key), "{key}");
    }
    let mut h = Headless::new();
    let fur = tmp("fur.png");
    h.call("engine.execute", json!({"command": "photo.syntheticFur", "params": {"path": fur, "width": 160, "height": 120}})).unwrap();
    let args = json!({"file": fur, "provider": "none", "widthMm": 50});
    let direct = h.call("engine.execute", json!({"command": "photo.toStitches", "params": args})).unwrap();
    let dst = tmp("fur.dst");
    let mut with_path = args.clone();
    with_path["path"] = json!(dst);
    let r = text(&call_tool(&mut h, "photo_to_stitches", &with_path));
    assert_eq!(r["stitches"], direct["stitches"], "{r}");
    assert_eq!((r["format"].as_str(), r["path"].as_str()), (Some("dst"), Some(dst.as_str())));
    assert!(std::fs::read(&dst).unwrap().starts_with(b"LA:"));
    // A bad option is a tool error.
    let r = call_tool(&mut h, "photo_to_stitches", &json!({"file": fur, "fill": "satin"}));
    assert!(r.is_error && r.content[0]["text"].as_str().unwrap().contains("fill"), "{r:?}");
}
