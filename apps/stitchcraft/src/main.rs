//! StitchCraft desktop app: the embroidery sibling of VectorCraft. It is VectorCraft's launcher
//! (`apps/vectorcraft/src/lib.rs`) on the same crates, started under StitchCraft's name, with its
//! own settings folder and icon, no update check, and the Stitch Properties panel docked on its
//! first start. See `docs/superpowers/specs/2026-11-11-stitchcraft-app-shell.md`.
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
    // The Stitch Properties and Stitch Preview panels (`vectorcraft_ui_egui::panels::stitch_props::ID`
    // and `panels::stitch_preview::ID`), each docked under Properties in that order.
    first_start_panels: &["stitchProps", "stitchPreview"],
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
    }

    /// The window title and the app bar name StitchCraft, and the panels of its first start open in order.
    #[test]
    fn the_window_says_stitchcraft_and_its_first_start_panels_open() {
        vectorcraft_ui_egui::set_app_brand(vectorcraft_ui_egui::AppBrand { name: STITCHCRAFT.name, mark_png: STITCHCRAFT.brand_png });
        let mut app = vectorcraft_ui_egui::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        assert_eq!(vectorcraft_ui_egui::chrome::window_title(&app), "StitchCraft");
        assert_eq!(vectorcraft_ui_egui::app_name(), "StitchCraft");
        assert_eq!(STITCHCRAFT.first_start_panels, [vectorcraft_ui_egui::panels::stitch_props::ID, vectorcraft_ui_egui::panels::stitch_preview::ID]);
        vectorcraft_app::identity::dock_first_start_panels(&mut app, &STITCHCRAFT);
        // Docked beside the canvas, in a group of its own under Properties: not floating.
        let layout = serde_json::to_value(&app.ui.docking).unwrap();
        assert_eq!(layout["floating"], serde_json::json!([]), "{layout}");
        let dock = &layout["root"]["Split"]["second"]["Dock"]["columns"][0]["groups"];
        assert_eq!(dock[0]["panels"][0], "properties", "{layout}");
        assert_eq!(dock[1]["panels"], serde_json::json!(["stitchProps"]), "{layout}");
        assert_eq!(dock[2]["panels"], serde_json::json!(["stitchPreview"]), "{layout}");
    }

    #[test]
    fn the_icons_are_stitchcrafts_own() {
        for png in [STITCHCRAFT.icon_macos_png, STITCHCRAFT.icon_png, STITCHCRAFT.brand_png] {
            assert!(png.starts_with(b"\x89PNG"), "a PNG");
        }
        assert_ne!(STITCHCRAFT.brand_png, vectorcraft_app::VECTORCRAFT.brand_png);
    }
}
