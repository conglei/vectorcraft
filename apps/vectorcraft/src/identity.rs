//! Which app the shared desktop launcher ([`crate::run`]) starts. VectorCraft and its embroidery
//! sibling StitchCraft (`apps/stitchcraft`) are the same program on the same crates; they differ in
//! what this names: the app's name, its settings folder, its icons, whether it looks for updates and
//! which panels open on its first start (`docs/superpowers/specs/2026-11-11-stitchcraft-app-shell.md`).

use std::sync::OnceLock;

/// What tells one Craft desktop app built on this launcher from another.
#[derive(Debug)]
pub struct AppIdentity {
    /// The name people see: window title, app bar, About, the macOS app menu, log lines and
    /// messages, and the settings folder on macOS and Windows.
    pub name: &'static str,
    /// The command name (`--version`, error messages) and the settings folder on Linux and the BSDs.
    pub lower: &'static str,
    /// The bundle identifier, which is also the Wayland app ID and X11 class (the desktop entry's
    /// name), so docks match the window to its launcher.
    pub bundle_id: &'static str,
    /// The control-channel port the documentation and `--help` give for this app. `--control`
    /// still takes the port it is given.
    pub default_control_port: u16,
    /// Whether the app looks for a newer release when it starts and on Help › Check for Updates.
    pub update_check: bool,
    /// The runtime Dock icon on macOS (the tile with Apple's transparent margin).
    pub icon_macos_png: &'static [u8],
    /// The runtime window icon elsewhere (the full-bleed 256 px tile).
    pub icon_png: &'static [u8],
    /// The 128 px brand mark in the app bar and the About box.
    pub brand_png: &'static [u8],
    /// The panels a first start shows (no saved preferences yet), in order.
    pub first_start_panels: &'static [&'static str],
}

/// VectorCraft, the vector illustration app.
pub const VECTORCRAFT: AppIdentity = AppIdentity {
    name: "VectorCraft",
    lower: "vectorcraft",
    bundle_id: "ai.storyteller.vectorcraft",
    default_control_port: 7979,
    update_check: true,
    icon_macos_png: include_bytes!("../../../assets/app-icon/vectorcraft-macos-512.png"),
    icon_png: include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.vectorcraft.png"),
    brand_png: include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png"),
    first_start_panels: &[],
};

static CURRENT: OnceLock<&'static AppIdentity> = OnceLock::new();

/// Make `identity` this process's app. The first call wins: one process is one app.
pub(crate) fn set(identity: &'static AppIdentity) {
    // A second call keeps the first app: it cannot change what has already been named.
    let _ = CURRENT.set(identity);
}

/// Show `app`'s first-start panels docked open: each as a group of its own beside the canvas, the
/// first under the Properties group and every next one under the one before it, so they read in
/// the order listed. `window.panel` alone would float a panel the default workspace has no place
/// for. A panel this build lacks is logged and skipped, leaving the workspace as it is.
pub fn dock_first_start_panels(ui: &mut vectorcraft_ui_egui::VectorcraftApp, app: &AppIdentity) {
    let mut below = "properties";
    for panel in app.first_start_panels {
        let shown = ui.run("window.panel", serde_json::json!({ "panel": panel }));
        let docked = shown.and_then(|_| ui.run("window.panel.dock", serde_json::json!({ "panel": panel, "below": below })));
        match docked {
            Ok(_) => below = panel,
            Err(e) => log::warn!("first start: panel {panel} not docked: {e}"),
        }
    }
}

/// This process's app: VectorCraft until [`set`] names another (and in unit tests).
pub(crate) fn current() -> &'static AppIdentity {
    CURRENT.get().copied().unwrap_or(&VECTORCRAFT)
}
