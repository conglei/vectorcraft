//! The VectorCraft brand mark: the app icon (the engraved dragon, `assets/app-icon/`, see its
//! README). The PNG carries the icon's rounded corners; it is decoded once per context into a
//! mipmapped texture, so it stays crisp from the 18 pt app-bar mark to the About box.

use egui::{Color32, Context, Id, Rect, TextureHandle, TextureOptions, Ui, pos2};

/// 128 px: sharp at 44 pt on a 2× display, and the mipmaps keep 22 pt clean.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.vectorcraft.png");

/// The app this UI runs in, as people see it: VectorCraft, or a sibling app built on the same
/// crates (StitchCraft) that names itself with [`set_app_brand`].
#[derive(Debug, Clone, Copy)]
pub struct AppBrand {
    /// The window title, the app bar's title, the About box and the macOS app menu.
    pub name: &'static str,
    /// The brand mark: a 128 px PNG of the app icon, with its rounded corners.
    pub mark_png: &'static [u8],
    /// The line under the welcome on the Home screen: what the app is for.
    pub tagline: &'static str,
}

/// The tagline of VectorCraft, which is also the default.
const TAGLINE: &str = "Vector illustration — fast, open, scriptable.";

static BRAND: std::sync::OnceLock<AppBrand> = std::sync::OnceLock::new();

/// Name the app this process runs, before its first frame. The first call wins: one process is
/// one app.
pub fn set_app_brand(brand: AppBrand) {
    // A later call keeps the first brand: the window may already show it.
    let _ = BRAND.set(brand);
}

fn brand() -> AppBrand {
    BRAND.get().copied().unwrap_or(AppBrand { name: "VectorCraft", mark_png: ICON_PNG, tagline: TAGLINE })
}

/// The app's name ([`set_app_brand`]; VectorCraft by default).
pub fn app_name() -> &'static str {
    brand().name
}

/// The line under the welcome on the Home screen ([`set_app_brand`]).
pub fn tagline() -> &'static str {
    brand().tagline
}

/// [`rename`] for a menu label, which stays as it is (without copying it) in VectorCraft and in a
/// label that doesn't name the app.
pub(crate) fn rename_label(label: std::borrow::Cow<'static, str>) -> std::borrow::Cow<'static, str> {
    rename_with(app_name(), label)
}

/// [`rename_label`] for the app called `name`.
fn rename_with(name: &str, label: std::borrow::Cow<'static, str>) -> std::borrow::Cow<'static, str> {
    if name == "VectorCraft" || !label.contains("VectorCraft") { label } else { label.replace("VectorCraft", name).into() }
}

/// `text` with VectorCraft's name replaced by the running app's ([`app_name`]): for the few labels
/// that name the app (About VectorCraft, Quit VectorCraft), in every language.
pub(crate) fn rename(text: &str) -> String {
    text.replace("VectorCraft", app_name())
}

/// The icon's pixels (one transparent pixel if it couldn't be decoded, which a test rules out).
fn decode() -> egui::ColorImage {
    match image::load_from_memory_with_format(brand().mark_png, image::ImageFormat::Png) {
        Ok(img) => {
            let img = img.to_rgba8();
            egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
        }
        Err(_) => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// The icon texture, uploaded on first use and kept in the context.
fn texture(ctx: &Context) -> TextureHandle {
    let id = Id::unique("vectorcraft-brand-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    let tex = ctx.load_texture("vectorcraft-brand-mark", decode(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Paint the mark into `r` (square).
pub fn paint_mark(ui: &Ui, r: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture(ui.ctx()).id(), r, uv, Color32::WHITE);
}

/// The mark's texture id (tests: find the painted marks among the shapes).
#[cfg(test)]
pub fn texture_id(ctx: &Context) -> egui::TextureId {
    texture(ctx).id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VectorcraftApp;

    #[test]
    fn a_label_that_names_the_app_names_the_running_one() {
        use std::borrow::Cow;
        let renamed = rename_with("StitchCraft", Cow::Borrowed("About VectorCraft"));
        assert_eq!(renamed, "About StitchCraft");
        // A label that doesn't name the app, and VectorCraft's own, are not copied.
        assert!(matches!(rename_with("StitchCraft", Cow::Borrowed("Save As…")), Cow::Borrowed(_)));
        assert!(matches!(rename_with("VectorCraft", Cow::Borrowed("About VectorCraft")), Cow::Borrowed(_)));
        // The default brand is VectorCraft's (the tests of this crate run under it).
        assert_eq!((app_name(), tagline()), ("VectorCraft", TAGLINE));
    }

    #[test]
    fn the_icon_decodes_to_a_rounded_128_px_tile() {
        let img = decode();
        assert_eq!(img.size, [128, 128]);
        // Rounded corners: the corner pixel is transparent, the centre opaque.
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[64 * 128 + 64].a(), 255);
    }

    #[test]
    fn the_app_bar_paints_the_icon_from_one_cached_texture() {
        let ctx = Context::default();
        crate::theme::install_fonts(&ctx);
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        // The system title bar hides the in-app mark (the OS draws its own icon), so paint the
        // app-drawn bar here.
        app.custom_titlebar = true;
        let mut frame = || {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::chrome::app_bar(&mut app, ui));
            let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
            out.textures_delta.clear();
            let marks: Vec<Rect> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Mesh(m) if m.texture_id == texture(&ctx).id() => Some(m.calc_bounds()),
                    _ => None,
                })
                .collect();
            (uploads, marks)
        };
        let (uploads, marks) = frame();
        assert_eq!(uploads, 1);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].size(), egui::vec2(18.0, 18.0));
        // Later frames reuse the texture.
        let (uploads, marks) = frame();
        assert_eq!((uploads, marks.len()), (0, 1));
    }
}
