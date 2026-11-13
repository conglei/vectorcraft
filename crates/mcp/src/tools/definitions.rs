use super::dispatch::{BRIEF, MAX_SIDE_WINDOW, MENU_FILTER, TRACE_VIEWS};
use super::handlers::MAX_DIALOG_FIELDS;
use super::{FORMATS, OPEN_EXTS, SAVE_FORMATS, Value, format, json};

// ---------- schemas ----------

fn num(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

/// The arguments `save_file` passes on to `document.save` beside `path`.
pub(super) const SAVE_FILE_OPTIONS: [&str; 5] = ["compress", "version", "preview", "precision", "options"];

/// The arguments `doc_inspect` / `inspect_document` pass on to `document.inspect`.
pub(super) const INSPECT_KEYS: [&str; 6] = ["depth", "childLimit", "ids", "kind", "limit", "offset"];

/// Their schema.
fn inspect_props() -> Value {
    json!({
        "depth": {"type": "integer", "minimum": 0, "description": "Child levels of the layer tree to include (default all; 0 = top layers with counts)"},
        "childLimit": {"type": "integer", "minimum": 0, "description": "Children shown per node (default all; a level that shows fewer reports childCount)"},
        "ids": {"type": "array", "maxItems": 10000, "items": {"type": "integer", "minimum": 0}, "description": "List just these nodes (any level) as `nodes` instead of the whole layer tree; an unknown id is an error"},
        "kind": {"type": "string", "description": "List every node of this panel label (Layer, Group, Path, Type, Image, ...; ignores case) as `nodes`, each with its ancestor `path`; searches the subtrees of `ids` when given"},
        "limit": {"type": "integer", "minimum": 0, "description": "At most this many layers or nodes (the reply adds `total`)"},
        "offset": {"type": "integer", "minimum": 0, "description": "Layers or nodes skipped first (default 0)"}
    })
}

fn paint_schema(what: &str) -> Value {
    json!({
        "description": format!("{what}: \"#rrggbb\", \"none\", [r,g,b] (0..1), {{\"c\",\"m\",\"y\",\"k\"}}, {{\"gray\"}}, or a full paint.setFill params object ({{\"gradient\":…}} / {{\"swatch\":name}})"),
        "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "number"}}, {"type": "object"}]
    })
}

/// The options `draw_path` and `draw_shape` apply to the object they draw (#2112).
fn with_draw_options(mut props: Value) -> Value {
    let more = json!({
        "dash": {"description": "Dash and gap lengths in pt ([d, g, …]), or null for a solid stroke (stroke.set `dash`)", "anyOf": [{"type": "array", "items": {"type": "number"}}, {"type": "null"}]},
        "cap": {"type": "string", "enum": ["butt", "round", "square", "projecting"], "description": "Stroke cap"},
        "join": {"type": "string", "enum": ["miter", "round", "bevel"], "description": "Stroke corner join"},
        "opacity": {"type": "number", "minimum": 0, "maximum": 100, "description": "Opacity in % (object.setProps `opacity`); doc_inspect reads it back as `opacityPct` (its `opacity` is a 0–1 fraction)"},
        "name": string("Name in the Layers panel"),
        "effects": {
            "type": "array",
            "description": "Live effects applied in order, as apply_effect: [{effect: id (effect.list), params?: {…}}]",
            "items": obj(json!({"effect": string("Effect id, e.g. \"blur.gaussian\""), "params": {"type": "object"}}), &["effect"]),
        },
    });
    if let (Some(p), Value::Object(more)) = (props.as_object_mut(), more) {
        p.extend(more);
    }
    props
}

/// The stitch options of `stitch.info` and of a stitch file export (lengths in mm), as `stitch_info`
/// and `export_stitch` take them, plus `format` (`default` says what it is when not given).
fn stitch_props(format: &str) -> Value {
    json!({
        "format": {"type": "string", "enum": ["dst", "dsb", "exp", "u01", "tbf", "pes", "pec"], "description": format},
        "machine": {"type": "string", "enum": ["commercial15", "generic"], "description": "commercial15 (default; 15 needles, a 500 x 350 mm field) or generic (no needle or field limits)"},
        "mode": {"type": "string", "enum": ["run", "manual"], "description": "run: running stitch along each stroked path; manual: one needle point per anchor. Not given: art on the layer an opened stitch file made is manual (its needle points export unchanged), all other art is run"},
        "stitchLength": {"type": "number", "minimum": 0.5, "maximum": 12, "description": "Running stitch length in mm (default 2.5)"},
        "minStitch": {"type": "number", "minimum": 0, "maximum": 2, "description": "Stitches shorter than this (mm) are dropped (default 0.3)"},
        "maxStitch": {"type": "number", "minimum": 1, "maximum": 12.7, "description": "The longest stitch in mm: 1-12.1 for DST, PES and PEC (default 12.1), 1-12.7 for EXP, DSB, U01 and TBF (default 12.7); longer ones are split by jumps"},
        "locks": {"type": "boolean", "description": "Lock stitches at the start and end of each run. Not given: off when everything stitched is on an opened stitch file's layer, on otherwise"},
        "jumpsPerTrim": {"type": "integer", "minimum": 2, "maximum": 8, "description": "Jumps in a row the machine reads as a trim (default 3)"},
        "trimBeforeColorChange": {"type": "boolean", "description": "A trim before each colour change (default false)"},
        "start": {"type": "string", "enum": ["center", "firstStitch"], "description": "The file's origin (default center)"},
        "returnToStart": {"type": "boolean", "description": "End the file with travel back to its origin (default true)"},
        "name": string("The design name in the file (default: the document's title)"),
        "selectedOnly": {"type": "boolean", "description": "Stitch only the selected objects (default false)"},
    })
}

/// The options of `stitch_worksheet` and `export_production`: the stitch options without `format`
/// (and `name`, which `export_production` takes as the file name), plus the estimate and hoop ones.
fn production_props() -> Value {
    let mut props = stitch_props("");
    if let Some(p) = props.as_object_mut() {
        p.remove("format");
        p.insert(
            "machine".into(),
            json!({"type": "string", "enum": ["commercial15", "bai_mirror", "generic"], "description": "commercial15 (default; 15 needles, 500 x 350 mm), bai_mirror (the owner's 15-needle machine: 1,200 stitches per minute, reads DST and DSB) or generic"}),
        );
        p.insert("spm".into(), json!({"type": "number", "minimum": 100, "maximum": 3000, "description": "Stitches per minute for the run time (default: the machine's top speed, else 800)"}));
        p.insert("hoop".into(), string("A hoop id of run_command stitch.hoops.list (default: the document's hoop, else the smallest that fits)"));
        p.insert(
            "marginMm".into(),
            json!({"type": "number", "minimum": 0, "maximum": 100, "description": "The hoop fit's clearance in mm (default 10)"}),
        );
    }
    props
}

fn mods_schema() -> Value {
    json!({
        "type": "object",
        "description": "Modifier keys held (cmd = Command on macOS / Ctrl elsewhere; alt = Option)",
        "properties": {"shift": {"type": "boolean"}, "alt": {"type": "boolean"}, "cmd": {"type": "boolean"}, "ctrl": {"type": "boolean"}},
        "additionalProperties": false
    })
}

fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

/// The extension of each format Save writes, in [`SAVE_FORMATS`] order.
fn save_extensions() -> Vec<&'static str> {
    SAVE_FORMATS.iter().filter_map(|id| format(id)?.extensions.first().copied()).collect()
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    let writes_file =
        matches!(name, "screenshot" | "save_file" | "export" | "export_stitch" | "export_production" | "stitch_worksheet" | "photo_to_stitches");
    let read_only = read_only && !writes_file;
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {"title": title, "readOnlyHint": read_only, "destructiveHint": !read_only, "idempotentHint": read_only, "openWorldHint": false},
    })
}

/// All tools, in `tools/list` order.
pub fn tool_definitions() -> Vec<Value> {
    let empty = || obj(json!({}), &[]);
    vec![
        tool(
            "command_list",
            "List commands",
            "Discover command ids, labels, parameter descriptions and enabled state. A command with a parameter schema also lists it as schema (JSON Schema 2020-12) and as inputSchema (the same schema with an object root, the form of a tool's inputSchema). Returns an array. The whole catalogue is large: narrow it with filter (or query), menu, and brief. list_commands lists the same commands; it takes enabledOnly instead of enabled_only and returns {count, commands}.",
            obj(
                json!({"filter": string("Case-insensitive id, label or menu substring"), "query": string("Same as filter"), "menu": string(MENU_FILTER), "enabled_only": {"type":"boolean"}, "brief": {"type":"boolean", "description": BRIEF}}),
                &[],
            ),
            true,
        ),
        tool(
            "command_run",
            "Run a command",
            "Run an engine command by id with its JSON params. Command parameters follow the registry's descriptions. run_command runs it the same way; it takes the id as command instead of id.",
            obj(json!({"id": string("Command id"), "params": {"type":"object"}}), &["id"]),
            false,
        ),
        tool(
            "command_batch",
            "Run several commands",
            "Run steps in order; each edit is its own undo step, or with atomic:true the whole batch is one (named `label`, else after its first edit; `atomic` says which documents). A step's id is a command id, or draw_path / draw_shape with that tool's arguments as params (its result is the tool's: the new object's `id`). A param string that is exactly \"$N.path\" (N: 0-based step index) or \"$last.path\" takes that value from an earlier step's result, e.g. {\"id\":\"$1.id\"}; \"$$\" starts a literal \"$\". Returns completed, failed, results and created (the ids draw_path / draw_shape steps made, in order). Stops on the first error by default. At most 256 steps; kept step results total at most 8 MiB, past which the batch stops with the reason.",
            obj(
                json!({"steps": {"type":"array", "maxItems": 256, "items":obj(json!({"id":string("Command id, or draw_path / draw_shape"),"params":{"type":"object"}}), &["id"])}, "stop_on_error":{"type":"boolean", "default":true}, "atomic":{"type":"boolean", "default":false, "description":"Make the batch's edits one undo step in the active document and in each document a step opens or creates; in a document open before the batch that a step switches to, each edit stays its own undo step"}, "label":string("The undo step's name with atomic")}),
                &["steps"],
            ),
            false,
        ),
        tool(
            "doc_inspect",
            "Inspect document",
            "Summary of the active document, including artboards, layer tree, selection and history. A node below full opacity reports `opacity` as a 0–1 fraction and `opacityPct` in the 0–100 % scale the drawing tools and object.setProps take: pass `opacityPct` back, never `opacity`. On a large document scope it: depth 0 for the layer skeleton, ids for just those nodes, kind for every node of one kind, limit/offset to page. inspect_document returns the same summary.",
            obj(inspect_props(), &[]),
            true,
        ),
        tool(
            "render_preview",
            "Render a preview",
            "Render an artboard (default the first) as inline PNG without changing the document. max_side defaults to 1024, limited to 1..4096.",
            obj(
                json!({"max_side":{"type":"integer","minimum":1,"maximum":4096},"artboard":{"type":"integer","minimum":0,"description":"0-based artboard (default 0)"},"traceViews":{"type":"boolean","description":TRACE_VIEWS}}),
                &[],
            ),
            true,
        ),
        tool(
            "ui_inspect",
            "Inspect the interface",
            "Inspect the connected desktop interface; returns a tool error in headless mode. The same call as inspect_ui.",
            empty(),
            true,
        ),
        tool(
            "ui_screenshot",
            "Capture the interface",
            "Capture the connected desktop window as inline PNG; requires a running desktop app. The same call as screenshot with window:true. max_side scales the returned image down to at most that many pixels on its longer side (1..4096; default: the full capture).",
            obj(json!({"max_side": {"type":"integer","minimum":1,"maximum":4096,"description":MAX_SIDE_WINDOW}}), &[]),
            true,
        ),
        tool(
            "list_commands",
            "List commands",
            "List VectorCraft commands (id, label, menu path, shortcut, parameter description, enabled state, and schema and inputSchema for a command with a parameter schema). Every editing action is a command; run any of them with run_command. Coordinates are points in document space, y down, origin at the first artboard's top-left. command_list lists the same commands as a bare array and takes enabled_only.",
            obj(
                json!({"filter": string("Case-insensitive substring matched against id, label and menu path (e.g. \"align\", \"Object\")"), "query": string("Same as filter"), "menu": string(MENU_FILTER), "enabledOnly": {"type": "boolean", "description": "Only commands that can run right now"}, "brief": {"type": "boolean", "description": BRIEF}}),
                &[],
            ),
            true,
        ),
        tool(
            "run_command",
            "Run command",
            "Execute any VectorCraft command by id with JSON params (see list_commands for ids and params). Examples: {\"command\":\"object.group\"}, {\"command\":\"object.align\",\"params\":{\"horizontal\":\"left\"}}, {\"command\":\"file.new\",\"params\":{\"width\":800,\"height\":600}}. Returns the command's result (e.g. {id} for creation commands). command_run runs it the same way and takes the id as id.",
            obj(
                json!({"command": string("Command id, e.g. shape.rectangle, object.group, paint.setFill"), "params": {"type": "object", "description": "Command parameters"}}),
                &["command"],
            ),
            false,
        ),
        tool(
            "inspect_document",
            "Inspect document",
            "Summary of the active document: artboards, layer tree (ids, names, kinds, bounds, fill/stroke; `opacity` as a 0–1 fraction and `opacityPct` in the 0–100 % scale the drawing tools take), selection, undo history, current tool and default paint. On a large document pass depth: 0 for the layer skeleton (truncated levels report childCount), then locate objects with run_command document.find and read them with document.node. doc_inspect returns the same summary.",
            obj(inspect_props(), &[]),
            true,
        ),
        tool(
            "inspect_ui",
            "Inspect UI",
            "UI state of the running desktop app: tool and options, panels, dialogs, view (zoom/center), window, documents, perf. Desktop app only. The same call as ui_inspect.",
            empty(),
            true,
        ),
        tool(
            "select_tool",
            "Select tool",
            "Switch the active tool (as clicking it in the Tools panel). Ids: selection, directSelection, groupSelection, pen, rectangle, roundedRectangle, ellipse, polygon, star, lineSegment, …",
            obj(json!({"tool": string("Tool id")}), &["tool"]),
            false,
        ),
        tool(
            "pointer_gesture",
            "Pointer gesture",
            "Drive the active tool with mouse events in document coordinates, exactly like the mouse would (e.g. a rectangle drag: down at (100,100), drag to (200,150), up at (200,150)). Optionally selects `tool` first.",
            obj(
                json!({
                    "tool": string("Optional tool to select before the gesture"),
                    "events": {
                        "type": "array",
                        "minItems": 1,
                        "items": obj(json!({
                            "kind": {"type": "string", "enum": ["down", "drag", "up", "move", "doubleclick"]},
                            "x": num("Document x (pt)"),
                            "y": num("Document y (pt)"),
                            "mods": mods_schema(),
                            "backquote": {"type": "boolean", "description": "Backquote held for this event: Selection and transform tool drags transform only pattern paint"},
                            "pressure": num("Pen pressure 0..1 (default 1): the Liquify tools' intensity with Use Pressure Pen on, and the pressure the Paintbrush records"),
                            "tiltX": num("Pen tilt in degrees −90..90, positive leaning right (document axes); with tiltY, the tilt the Paintbrush records for Tilt and Bearing brushes. One without the other is 0"),
                            "tiltY": num("Pen tilt in degrees −90..90, positive leaning down the page"),
                            "rotation": num("Pen barrel rotation in degrees clockwise: the rotation the Paintbrush records for Rotation brushes"),
                            "holdMs": num("Milliseconds the pointer then holds still, button down (0..60000): Twirl, Pucker and Bloat keep applying"),
                        }), &["kind", "x", "y"]),
                    },
                    "mods": mods_schema(),
                    "backquote": {"type": "boolean", "description": "Default backquote held state for events: Selection and transform tool drags transform only pattern paint"},
                }),
                &["events"],
            ),
            false,
        ),
        tool(
            "draw_path",
            "Draw path",
            "Create a path from points ([[x,y],…] or [{x,y,in?:[x,y],out?:[x,y]},…] with Bézier handles) or SVG path data `d`, then apply optional paint, stroke dash/cap/join, opacity, name and live effects. Omitted paint fields inherit the current new-art appearance (initially white fill and 1 pt black stroke). For a filled path without an outline, pass an explicit fill and stroke:\"none\". Returns the new object id.",
            obj(
                with_draw_options(json!({
                    "points": {"type": "array", "description": "Anchors: [x,y] pairs or {x, y, in?, out?, smooth?} objects", "items": {"anyOf": [{"type": "array", "items": {"type": "number"}, "minItems": 2}, {"type": "object"}]}},
                    "d": string("SVG path data (alternative to points), e.g. \"M0 0 L100 0 L50 80 Z\""),
                    "closed": {"type": "boolean", "description": "Close the path (points only)"},
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                })),
                &[],
            ),
            false,
        ),
        tool(
            "draw_shape",
            "Draw shape",
            "Create a live shape and apply optional paint. rectangle/ellipse: x, y, width, height (rectangle: radius = corner radius). polygon: cx, cy, radius, sides. star: cx, cy, radius1, radius2, points. line: x1, y1, x2, y2. Omitted paint fields inherit the current new-art appearance (initially white fill and 1 pt black stroke); lines start with no fill and a visible stroke. For a filled shape without an outline, pass an explicit fill and stroke:\"none\". Stroke dash/cap/join, opacity, name and live effects apply to the new shape as in draw_path. Returns the new object id.",
            obj(
                with_draw_options(json!({
                    "shape": {"type": "string", "enum": ["rectangle", "ellipse", "polygon", "star", "line"]},
                    "x": num("Left (rectangle/ellipse)"),
                    "y": num("Top (rectangle/ellipse)"),
                    "width": num("Width (rectangle/ellipse)"),
                    "height": num("Height (rectangle/ellipse)"),
                    "radius": num("Corner radius (rectangle) or radius (polygon)"),
                    "cx": num("Centre x (polygon/star)"),
                    "cy": num("Centre y (polygon/star)"),
                    "sides": {"type": "integer", "minimum": 3, "description": "Polygon sides (default 6)"},
                    "radius1": num("Star outer radius"),
                    "radius2": num("Star inner radius"),
                    "points": {"type": "integer", "minimum": 3, "description": "Star points (default 5)"},
                    "rotation": num("Rotation in degrees (polygon/star)"),
                    "x1": num("Line start x"),
                    "y1": num("Line start y"),
                    "x2": num("Line end x"),
                    "y2": num("Line end y"),
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                })),
                &["shape"],
            ),
            false,
        ),
        tool(
            "set_paint",
            "Set paint",
            "Set fill, stroke and/or stroke weight on the selection (or `ids`) and as the default for new art. Returns {applied, objects}: `objects` counts the objects painted, as paint.setFill, paint.setStroke and stroke.set report them (the largest of their counts); when it is 0, a `note` says that only the default for new art changed.",
            obj(
                json!({
                    "fill": paint_schema("Fill"),
                    "stroke": paint_schema("Stroke"),
                    "strokeWidth": num("Stroke weight in pt"),
                    "ids": {"type": "array", "items": {"type": "integer"}, "description": "Object ids (default: the selection)"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "press_key",
            "Press key",
            "Send a key press with modifiers, e.g. {key:\"Z\", mods:{cmd:true}} (undo), {key:\"Escape\"}, {key:\"Enter\"}. In the desktop app it goes through the real keyboard path; headless, it runs the command or tool bound to that shortcut.",
            obj(
                json!({"key": string("Key name: A–Z, 0–9, Enter, Escape, Delete, Backspace, Tab, Space, Left/Right/Up/Down, [ ]…"), "mods": mods_schema()}),
                &["key"],
            ),
            false,
        ),
        tool(
            "type_text",
            "Type text",
            "Type text into the focused widget (text tool, dialog field). Desktop app only.",
            obj(json!({"text": string("Text to type")}), &["text"]),
            false,
        ),
        tool(
            "answer_dialog",
            "Answer dialog",
            "Set fields of the desktop app's open dialog in order, then confirm (OK) or cancel it, as the control protocol's ui.dialog.set, ui.dialog.confirm and ui.dialog.cancel do. The app stores all the fields or none and answers that same dialog in the same request (ui.dialog.set with action). When the app refuses the ui.dialog.set, for example because no dialog is open, the call is an error and the dialog is not confirmed or canceled. inspect_ui shows the open dialog and its fields (ui.dialog). After invoke_menu app.quit or file.close replies {\"pending\":\"saveChanges\"}: {action:\"confirm\"} saves, {set:[{field:\"discard\",value:true}],action:\"confirm\"} closes without saving, {action:\"cancel\"} cancels. Returns {dialog, result}: the dialog after the fields were set, and the reply to confirm or cancel. Desktop app only.",
            obj(
                json!({
                    "set": {
                        "type": "array",
                        "maxItems": MAX_DIALOG_FIELDS,
                        "items": obj(json!({"field": string("Dialog field"), "value": {"description": "The field's new value (any JSON; default null)"}}), &["field"]),
                        "description": "Fields to set, in order (one ui.dialog.set that sets all of them or none)",
                    },
                    "action": {"type": "string", "enum": ["confirm", "cancel"], "description": "Then confirm (OK) or cancel the dialog the fields were set in"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "invoke_menu",
            "Invoke menu",
            "Invoke a menu item by its command id (same ids as list_commands, including UI-only commands like view.zoomIn or window.* in the desktop app).",
            obj(json!({"command": string("Menu command id"), "params": {"type": "object"}}), &["command"]),
            false,
        ),
        tool(
            "open_panel",
            "Open panel",
            "Open a panel in the desktop app's dock by id (layers, swatches, stroke, align, pathfinder, transform, …; case-insensitive, display labels like \"Layers\" work too). Desktop app only.",
            obj(json!({"panel": string("Panel id or display label")}), &["panel"]),
            false,
        ),
        tool(
            "screenshot",
            "Screenshot",
            "Render the active artboard to PNG and return it as an image (optionally also saving it to `path`). With window:true (desktop app only) captures the whole app window instead, as ui_screenshot does; max_side then scales the returned image down (a file written to `path` keeps the full capture).",
            obj(
                json!({
                    "path": string("Also write the PNG here"),
                    "scale": num("Pixels per point (default 1)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "Artboard index (default 0)"},
                    "window": {"type": "boolean", "description": "Capture the app window (desktop app only)"},
                    "max_side": {"type": "integer", "minimum": 1, "maximum": 4096, "description": MAX_SIDE_WINDOW},
                    "traceViews": {"type": "boolean", "description": TRACE_VIEWS},
                }),
                &[],
            ),
            true,
        ),
        tool(
            "open_file",
            "Open file",
            &format!(
                "Open a file as a new, active document: .{} (see run_command document.formats). Templates (.ait, native templates) open as a new untitled document. The reply's `warnings` say what didn't come in as it was (an EPS shown as its preview image says why its PostScript couldn't be read). A file of an extension no built-in format reads opens with the installed importer plug-in that lists it; `importer` names the importer to use (File › Open With), also for an extension a built-in format reads, and `params` gives that importer's parameters.",
                OPEN_EXTS.join(", .")
            ),
            obj(
                json!({
                    "path": string("File path"),
                    "importer": string("An importer plug-in's format id, plugin.<id> (see run_command document.formats), which must list the file's extension (default: the built-in format that reads the file, else the first importer by id that lists its extension)"),
                    "params": {"type": "object", "description": "The importer plug-in's parameters (see run_command plugin.list; default: the defaults its manifest declares)"},
                }),
                &["path"],
            ),
            false,
        ),
        tool(
            "save_file",
            "Save file",
            &format!(
                "Save the active document with the engine's document.save: by default to its own file in its own format (native .vectorcraft unless it was opened from or saved as SVG, PDF or a restorable .ai). With `path`, the extension picks the format: .{}. Other formats are exports (see export). A native file is gzipped when `compress` is true (default: the useCompression preference, Preferences > File Handling > Use Compression); `version` writes it for an older VectorCraft, `preview` embeds a PNG of the first artboard, and `precision` rounds path anchors and handles to that many decimal places for that save only. `options` carries the format's save options (see run_command document.formats); giving `compress`, `version`, `preview` or `precision` both at the top level and in `options` with different values is an error. The reply's `format` says what was written and `warnings` what that format loses and each option given that the format doesn't read (such as `compress` for .svg). In the running app the call returns once the file is written, or with the error, even with Background Save on. When the save asks first (before replacing a file that opened with losses, or with a file dialog for an untitled document without `path`), the reply is `pending` or \"cancelled\" as before.",
                save_extensions().join(", .")
            ),
            obj(
                json!({
                    "path": string("Destination; its extension picks the format (default: the document's own file)"),
                    "compress": {"type": "boolean", "description": "Native files: gzip the file (default: the useCompression preference)"},
                    "version": {"type": "integer", "minimum": 1, "maximum": vectorcraft_format::VERSION, "description": "Native files: the format version to write (default: the current one; 1 and 2 are never compressed)"},
                    "preview": {"type": "boolean", "description": "Native files: embed a PNG of the first artboard, at most 256 px (default false)"},
                    "precision": {"type": "integer", "minimum": 0, "maximum": vectorcraft_format::MAX_PRECISION, "description": "Native files: round path anchors and handles to this many decimal places, a smaller file; colors, transforms and type stay exact (default: exact)"},
                    "options": {"type": "object", "description": "The format's save options, passed through (see run_command document.formats); default: as last saved"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "export",
            "Export",
            "Export the active document with the engine's document.export (the same bytes in the app and headless), in any format of the `format` enum: vectors (svg/svgz with the artboard as viewBox, eps, dxf, emf, wmf), pdf and ai (one page per artboard: all, or `artboard` / `range`; ai is the PDF-compatible .ai file that Save writes, which carries the document when every artboard is written in order), rasters (one rendered artboard: png, jpg, webp, gif, png8 as an indexed .png, tiff, bmp, tga, psd with its layers), txt (the stories, back to front), embroidery stitch files (dst, dsb, exp, u01, tbf, pes, pec: the stroked paths as running stitch; the options of stitch.info), vectorcraft or template (a native copy or template; the document keeps its path). run_command document.formats lists each format's options. For SVG/SVGZ, top-level `outlineText: true` writes text as paths; DXF takes it too (its type is outlined anyway unless `preserve: editability`). PDF instead uses `options.advanced.outlineText: true`, default false (live text). `selection: true` exports only the selected objects, cropped to their bounds. Template layers are left out; live effects are kept (geometry baked, SVG filters for shadows/glows/blur). Without `path` the bytes come back as dataBase64.",
            obj(
                json!({
                    "format": {"type": "string", "enum": FORMATS.iter().filter(|f| f.write).map(|f| f.id).collect::<Vec<_>>(), "description": "Default: from the path's extension"},
                    "path": string("Destination file (omit to get dataBase64)"),
                    "scale": num("Raster pixels per point (without scale or options.ppi, uses the document's raster effects resolution; options.ppi wins)"),
                    "artboard": {"type": "integer", "minimum": 0, "description": "0-based artboard (default: the first; PDF and .ai default: all)"},
                    "range": string("1-based artboards such as \"1-3, 5\" (PDF and .ai: one page each; SVG and SVGZ: one file each; EPS: one file each, after a central file of the whole document when there are two or more and art shows; rasters, DXF, EMF and WMF: one, or one file each with options.useArtboards: true)"),
                    "selection": {"type": "boolean", "description": "Export only the selection, cropped to it"},
                    "outlineText": {"type": "boolean", "description": "SVG/SVGZ: true writes text as outlines (viewable without the fonts). DXF takes it too (type outlined anyway unless preserve: editability). PDF uses options.advanced.outlineText instead (default false: live text)"},
                    "options": {"type": "object", "description": "More format options, passed through (see run_command document.formats), e.g. {\"quality\": 80} for jpg or {\"advanced\": {\"outlineText\": true}} for PDF text as outlines (default false: live text)"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "stitch_info",
            "Stitch info",
            "What a DST or EXP export of the active document would write, without writing a file: the stitches, jumps, trims, colour changes, records in the file, the size in mm, one entry per thread (rgb, stitches, lengthMm, in sewing order) and warnings. It runs the stitch.info command, so the options are the export's; a document with no stroked paths is an error. Check it before export_stitch.",
            obj(stitch_props("dst (default) or exp"), &[]),
            true,
        ),
        tool(
            "export_stitch",
            "Export a stitch file",
            "Write the active document's stroked paths as an embroidery stitch file (running stitch, one thread per stroke colour) with the options of stitch_info. It runs document.export, so run_command document.formats lists the options too. Without `path` the bytes come back as dataBase64; the reply carries `warnings`. Export only after stitch_info shows sensible counts and no warnings you cannot accept.",
            obj(
                {
                    let mut props = stitch_props(
                        "dst, dsb, exp, u01 (Barudan, with needle numbers), tbf (Tajima, with needle numbers), pes or pec (Brother). Default: from the path's extension, else dst",
                    );
                    if let Some(p) = props.as_object_mut() {
                        p.insert("path".into(), string("Destination file (omit to get dataBase64)"));
                    }
                    props
                },
                &[],
            ),
            false,
        ),
        tool(
            "stitch_worksheet",
            "Production worksheet",
            "Write a one-page A4 PDF production worksheet for the active document's embroidery: the design picture, size, stitch count, colour changes, trims, hoop, machine, speed and estimated run time, top and bobbin thread, the design check's count, and the stop table (thread colour, code and name, needle, stitches, length per colour block). It runs the stitch.worksheet command; check the design first with run_command stitch.check.",
            obj(
                {
                    let mut props = production_props();
                    if let Some(p) = props.as_object_mut() {
                        p.insert("path".into(), string("The PDF file to write"));
                    }
                    props
                },
                &["path"],
            ),
            false,
        ),
        tool(
            "export_production",
            "Export for production",
            "Write everything the machine operator needs into one folder: the machine files (default: the formats the machine's panel reads, DST and DSB on bai_mirror, else DST) and <name>-worksheet.pdf. It runs stitch.exportProduction and returns the files written with their sizes, the design check's findings (errors, warnings, info) and the files' warnings. Files of the same names in the folder are replaced.",
            obj(
                {
                    let mut props = production_props();
                    if let Some(p) = props.as_object_mut() {
                        p.insert("dir".into(), string("The folder to write into (made if missing)"));
                        p.insert("name".into(), string("The files' base name: letters, digits, spaces, - _ and . (default: the design name)"));
                        p.insert("formats".into(), json!({"type": "array", "items": {"type": "string", "enum": ["dst", "dsb", "exp"]}, "description": "The machine files to write (default: the machine's panel formats, else dst)"}));
                        p.insert("worksheet".into(), json!({"type": "boolean", "description": "Also write the worksheet PDF (default true)"}));
                    }
                    props
                },
                &["dir"],
            ),
            false,
        ),
        tool(
            "photo_to_stitches",
            "Photo to stitches",
            "Turn a photo (a pet's fur, say) into an embroidery design in a new document: thread colours from a chart, colour regions, a direction field from the photo, and fills whose rows follow it (or straight tatami rows at each region's mean direction), sewn colour by colour. It runs the photo.toStitches command; with `path` it then writes the stitch file with document.export (format from the extension: dsb or exp, else dst). Returns the stitch, jump, trim and colour-change counts, the thread colours with their coverage, the regions and warnings. Works on this computer unless provider is gemini (or the photoProvider preference chooses it), which sends the photo to Google's Gemini API. See run_command photo.palette, photo.regions and photo.directionField for the stages alone.",
            obj(
                json!({
                    "file": string("The photo: a PNG, JPEG, WebP, GIF, TIFF or BMP file, at most 64 MB"),
                    "image": string("The photo as base64, instead of file"),
                    "provider": {"type": "string", "enum": ["none", "nomodel", "gemini"], "description": "Prepare the photo first: none (as it is), nomodel (on this computer) or gemini (sends it to Google's Gemini API, which cuts the subject out). Default: the photoProvider preference (nomodel unless chosen)"},
                    "colors": {"type": "integer", "minimum": 7, "maximum": 16, "description": "Thread colours (default 8)"},
                    "chart": string("Thread chart id or name (stitch.threads.list; default the active chart, else the starter set)"),
                    "maxSide": num("Pixels the photo is worked at, 64–2048 (default 600)"),
                    "widthMm": num("Design width in mm, 10–1000 (default 100)"),
                    "heightMm": num("Design height in mm, 10–1000 (with widthMm: fits within both)"),
                    "fill": {"type": "string", "enum": ["field", "tatami", "rows", "meander"], "description": "field (default): rows follow the photo's direction field; tatami: straight rows at each region's mean direction, regions left editable; rows: one thread, rows zigzagging wider and in shorter stitches where darker; meander: one thread, one open path that never crosses itself, finer where darker"},
                    "spacing": num("mm between rows, 0.2–5 (default 0.4; rows and meander 1: the rows' pitch or the meander's finest cell)"),
                    "color": string("rows and meander: the thread colour as #rrggbb, matched to the chart (default the chart's darkest)"),
                    "mergeGapMm": num("mm 0–3 (default 0.5): gaps of another thread this narrow between areas of one thread are closed"),
                    "minRegionMm2": num("mm², 0–10000 (default 2): smaller sewn areas are absorbed into the neighbour with the longest shared border"),
                    "travelMm": num("mm 0–12 (default 3): gaps within a colour up to this are travel (stitched inside the colour, else one jump), not a trim"),
                    "features": {"type": "boolean", "description": "field: sew small high-contrast features (eyes, a nose) last, over the fills (default true)"},
                    "minLength": num("Shortest stitch, mm 0.5–12 (default 2)"),
                    "maxLength": num("Longest stitch, mm 0.5–12 (default 4): lengths are random between the two"),
                    "feather": num("mm 0–10 (default 0): row ends pulled in by a random amount up to this, a ragged fur edge"),
                    "featherEdges": {"type": "string", "enum": ["both", "start", "end"], "description": "Which row ends are feathered (default both)"},
                    "seed": {"type": "integer", "minimum": 0, "description": "Random seed (default 1): the same seed sews the same"},
                    "minArea": num("mm², 0–10000 (default 4): smaller patches join their neighbours"),
                    "detail": num("0–100 (default 40): how closely region outlines follow the photo"),
                    "blur": num("Pixels, 0–8 (default 1.5): softens fine texture before colours are chosen"),
                    "cell": num("Direction field cell, mm 0.5–50 (default 2)"),
                    "smoothing": num("Direction field smoothing, mm 0.2–100 (default 2)"),
                    "maxStitches": {"type": "integer", "minimum": 1, "maximum": 1_000_000, "description": "More stitches is an error (default 100000)"},
                    "path": string("Write the stitch file here (omit to only build the document)"),
                    "format": {"type": "string", "enum": ["dst", "dsb", "exp", "u01", "tbf", "pes", "pec"], "description": "The stitch file format (default: from the path's extension, else dst)"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "add_text",
            "Add text",
            "Create point type at (x, y) (the first baseline), area type when width/height are given (`fit` autoHeight grows the frame to the text, shrinkText scales the text down to fit), or type in/on an existing path (`path` id + `mode` area|onPath; `pathEffect` rainbow|skew|3dRibbon|stairStep|gravity). Returns the text object id, with `overflow: true` when the text doesn't fit its frame or path (the lines past it aren't drawn: enlarge the frame, shrink the text or give `fit`).",
            obj(
                json!({
                    "text": {"type": "string"},
                    "x": num("Baseline origin x (area type: left; point type: where the line starts, or its centre or end with justify center/right)"),
                    "y": num("First baseline y (area type: top)"),
                    "width": num("Area type frame width"),
                    "height": num("Area type frame height"),
                    "vertical": {"type": "boolean", "description": "Vertical writing for point, area or path type (default false, horizontal), as text.create / text.createInPath"},
                    "fit": {"type": "string", "enum": ["none", "autoHeight", "shrinkText"], "description": "Area type: keep the frame (none), fit its height to the text (autoHeight) or scale overflowing text down (shrinkText)"},
                    "fitMinPercent": num("shrinkText: the smallest scale in percent, 10 to 100 (default 50)"),
                    "path": {"type": "integer", "description": "Existing path id to flow the text in (area) or along (onPath)"},
                    "mode": {"type": "string", "enum": ["area", "onPath"]},
                    "pathEffect": {"type": "string", "enum": ["rainbow", "skew", "3dRibbon", "stairStep", "gravity"]},
                    "size": num("Font size in pt"),
                    "font": {"type": "string", "description": "Font family"},
                    "color": paint_schema("Text colour (#rrggbb)"),
                    "justify": {"type": "string", "enum": ["auto", "left", "center", "right", "justifyAll"], "description": "Paragraph alignment (default auto: the start of the text's direction). Point type lines up on x: center centres each line on it, right ends it there"},
                }),
                &["text"],
            ),
            false,
        ),
        tool(
            "apply_effect",
            "Apply live effect",
            "Append a live effect (Effect menu) to the selection or `ids`. Omit `effect` to get the catalogue (ids, parameters, defaults). Examples: stylize.dropShadow {x, y, blur, opacity}, distort.roughen, warp.arc {bend}, path.offsetPath {offset}, pathfinder.add (on groups), blur.gaussian {radius}.",
            obj(
                json!({
                    "effect": {"type": "string", "description": "Effect id (see the catalogue)"},
                    "params": {"type": "object", "description": "Effect parameters; missing keys take the dialog defaults"},
                    "ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "pathfinder",
            "Pathfinder",
            "Combine the selected (or `ids`) objects destructively, back to front: unite, minusFront, intersect, exclude, divide, trim, merge, crop, outline, minusBack. For a non-destructive version apply the pathfinder.* effect to a group.",
            obj(
                json!({
                    "operation": {"type": "string", "enum": ["unite", "minusFront", "intersect", "exclude", "divide", "trim", "merge", "crop", "outline", "minusBack"]},
                    "ids": {"type": "array", "items": {"type": "integer"}},
                }),
                &["operation"],
            ),
            false,
        ),
        tool(
            "transform",
            "Transform",
            "Move, rotate, scale, reflect or shear the selection (or `ids`); several may be combined and run in that order. Angles in degrees (counter-clockwise), scale in %. `origin` [x,y] defaults to the selection centre; `copy` transforms a copy.",
            obj(
                json!({
                    "ids": {"type": "array", "items": {"type": "integer"}},
                    "dx": num("Move right"),
                    "dy": num("Move down"),
                    "rotate": num("Rotation in degrees"),
                    "scale": num("Uniform scale in %"),
                    "scaleX": num("Horizontal scale in %"),
                    "scaleY": num("Vertical scale in %"),
                    "reflect": {"type": "string", "enum": ["vertical", "horizontal"]},
                    "shear": num("Shear angle in degrees (horizontal axis)"),
                    "origin": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2},
                    "copy": {"type": "boolean"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "create_graph",
            "Create graph",
            "Create a graph (graph tools) in the plot rectangle. Data as `csv` (first row: empty cell then series names; then one row per category: label, values…) or `series`/`categories`/`rows`. An empty CSV cell or a null in `rows` is a blank value (no column; lines break around it); a number in straight quotes is a label. Edit later with run_command graph.setData / graph.setType.",
            obj(
                json!({
                    "type": {"type": "string", "enum": ["column", "stackedColumn", "bar", "stackedBar", "line", "area", "scatter", "pie", "radar"]},
                    "x": num("Plot left"),
                    "y": num("Plot top"),
                    "width": num("Plot width"),
                    "height": num("Plot height"),
                    "csv": {"type": "string"},
                    "series": {"type": "array", "items": {"type": "string"}},
                    "categories": {"type": "array", "items": {"type": "string"}},
                    "rows": {"type": "array", "items": {"type": "array", "items": {"type": ["number", "null"]}}},
                }),
                &["x", "y", "width", "height"],
            ),
            false,
        ),
        tool(
            "text_wrap",
            "Text wrap",
            "Make the selected (or `ids`) objects wrap objects: area type below them in the same layer flows around them (offset in pt, invert = flow inside). `release: true` removes the wrap.",
            obj(
                json!({
                    "ids": {"type": "array", "items": {"type": "integer"}},
                    "offset": num("Gap between text and object (default 6 pt)"),
                    "invert": {"type": "boolean"},
                    "release": {"type": "boolean"},
                }),
                &[],
            ),
            false,
        ),
        tool("undo", "Undo", "Undo the last change (Edit → Undo). redo reapplies it.", empty(), false),
        tool("redo", "Redo", "Redo the last undone change (Edit → Redo): the change the last undo took back.", empty(), false),
    ]
}
