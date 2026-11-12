//! StitchCraft desktop app: the embroidery sibling of VectorCraft. It is VectorCraft's launcher
//! (`apps/vectorcraft/src/lib.rs`) on the same crates, started under StitchCraft's name, with its
//! own settings folder and icon, no update check, documents that open showing their stitches, and
//! the Embroidery workspace in millimetres on its first start. See `docs/superpowers/specs/2026-11-11-stitchcraft-app-shell.md`.
//!
//! Usage: as `vectorcraft` (`stitchcraft --help`).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use vectorcraft_app::AppIdentity;

/// StitchCraft as the launcher knows it.
const STITCHCRAFT: AppIdentity = AppIdentity {
    name: "StitchCraft",
    lower: "stitchcraft",
    bundle_id: "ai.storyteller.stitchcraft",
    // VectorCraft's agents use 7979; StitchCraft's use its own port so both can run side by side.
    default_control_port: 7989,
    // There is no StitchCraft release feed yet. VectorCraft's feed would offer VectorCraft, and
    // installing that would replace this app, so StitchCraft never checks for updates.
    update_check: false,
    icon_macos_png: include_bytes!("../../../assets/app-icon/stitchcraft-macos-512.png"),
    icon_png: include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.stitchcraft.png"),
    brand_png: include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.stitchcraft.png"),
    // The Embroidery workspace (`vectorcraft_ui_egui::workspaces::EMBROIDERY`): the stitch panels
    // beside the canvas, no column of illustration panels.
    default_workspace: Some("Embroidery"),
    // Embroidery is measured in millimetres: hoops, stitch lengths, densities.
    default_unit: Some("Millimeters"),
    // Embroidery is this app's work: the canvas shows the stitches, not the drawing they come from.
    stitch_view: true,
};

fn main() -> std::process::ExitCode {
    vectorcraft_app::run(&STITCHCRAFT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stitchcraft_is_its_own_app() {
        assert_eq!(STITCHCRAFT.name, "StitchCraft");
        assert_ne!(STITCHCRAFT.bundle_id, vectorcraft_app::VECTORCRAFT.bundle_id);
        assert_ne!(STITCHCRAFT.default_control_port, vectorcraft_app::VECTORCRAFT.default_control_port);
        // No release feed of its own yet: checked when this compiles.
        const { assert!(!STITCHCRAFT.update_check) };
        // Its documents open showing their stitches; VectorCraft's show the art.
        const { assert!(STITCHCRAFT.stitch_view && !vectorcraft_app::VECTORCRAFT.stitch_view) };
    }

    /// The window title and the app bar name StitchCraft, and its first start shows the Embroidery
    /// workspace in millimetres.
    #[test]
    fn the_window_says_stitchcraft_and_its_first_start_is_the_embroidery_workspace() {
        vectorcraft_ui_egui::set_app_brand(vectorcraft_ui_egui::AppBrand { name: STITCHCRAFT.name, mark_png: STITCHCRAFT.brand_png });
        let mut app = vectorcraft_ui_egui::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(vectorcraft_ui_egui::chrome::window_title(&app), "StitchCraft");
        assert_eq!(vectorcraft_ui_egui::app_name(), "StitchCraft");
        assert_eq!(STITCHCRAFT.default_workspace, Some(vectorcraft_ui_egui::workspaces::EMBROIDERY));
        vectorcraft_app::identity::first_start(&mut app, &STITCHCRAFT);
        assert_eq!(app.ui.workspace, "Embroidery");
        // One column beside the canvas: Stitch Sequence, Stitch Properties with Properties, Layers.
        // Nothing floats, and there is no second column of icons.
        let layout = serde_json::to_value(&app.ui.docking).unwrap();
        assert_eq!(layout["floating"], serde_json::json!([]), "{layout}");
        let columns = layout["root"]["Split"]["second"]["Dock"]["columns"].as_array().unwrap();
        assert_eq!(columns.len(), 1, "{layout}");
        let groups = &columns[0]["groups"];
        assert_eq!(groups[0]["panels"], serde_json::json!(["stitchSequence"]), "{layout}");
        assert_eq!(groups[1]["panels"], serde_json::json!(["stitchProps", "properties"]), "{layout}");
        assert_eq!(groups[2]["panels"], serde_json::json!(["layers"]), "{layout}");
        // New documents are measured in millimetres.
        app.run("file.new", serde_json::json!({})).unwrap();
        assert_eq!(format!("{:?}", app.session.general_unit()), "Millimeters");
    }

    #[test]
    fn the_icons_are_stitchcrafts_own() {
        for png in [STITCHCRAFT.icon_macos_png, STITCHCRAFT.icon_png, STITCHCRAFT.brand_png] {
            assert!(png.starts_with(b"\x89PNG"), "a PNG");
        }
        assert_ne!(STITCHCRAFT.brand_png, vectorcraft_app::VECTORCRAFT.brand_png);
    }
}
