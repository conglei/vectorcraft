//! Which app the shared desktop launcher ([`crate::run`]) starts. VectorCraft and its embroidery
//! sibling StitchCraft (`apps/stitchcraft`) are the same program on the same crates; they differ in
//! what this names: the app's name, its settings folder, its icons, whether it looks for updates and
//! which workspace and unit its first start sets (`docs/superpowers/specs/2026-11-11-stitchcraft-app-shell.md`).

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
    /// The workspace a first start shows (no saved preferences yet); `None`: Essentials.
    pub default_workspace: Option<&'static str>,
    /// The General unit a first start sets (Preferences › Units, by its name there), which new
    /// documents are measured in; `None`: the preference's own default.
    pub default_unit: Option<&'static str>,
    /// Documents open in the stitch view (View › Stitches: the canvas shows the stitches an export
    /// would sew) instead of Preview.
    pub stitch_view: bool,
    /// The line under the welcome on the Home screen: what the app is for.
    pub tagline: &'static str,
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
    default_workspace: None,
    default_unit: None,
    stitch_view: false,
    tagline: "Vector illustration — fast, open, scriptable.",
};

static CURRENT: OnceLock<&'static AppIdentity> = OnceLock::new();

/// Make `identity` this process's app. The first call wins: one process is one app.
pub(crate) fn set(identity: &'static AppIdentity) {
    // A second call keeps the first app: it cannot change what has already been named.
    let _ = CURRENT.set(identity);
}

/// What a first start of `app` sets, where a later start brings back what the person left: its
/// workspace and its General unit. One this build lacks is logged and skipped, leaving the
/// default in place.
pub fn first_start(ui: &mut vectorcraft_ui_egui::VectorcraftApp, app: &AppIdentity) {
    if let Some(name) = app.default_workspace
        && let Err(e) = ui.run("window.workspace", serde_json::json!({ "name": name }))
    {
        log::warn!("first start: workspace {name} not shown: {e}");
    }
    if let Some(unit) = app.default_unit
        && let Err(e) = ui.session.execute("prefs.set", &serde_json::json!({ "key": "unitsGeneral", "value": unit }))
    {
        log::warn!("first start: unit {unit} not set: {e}");
    }
}

/// This process's app: VectorCraft until [`set`] names another (and in unit tests).
pub(crate) fn current() -> &'static AppIdentity {
    CURRENT.get().copied().unwrap_or(&VECTORCRAFT)
}
