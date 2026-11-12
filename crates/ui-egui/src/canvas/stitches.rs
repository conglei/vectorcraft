//! View › Stitches: the canvas shows the stitches an export would sew, where the art is and at the
//! canvas's zoom, on the fabric colour, instead of the art. The plan, the view settings and the
//! playback position are the Stitch Preview panel's ([`crate::panels::stitch_preview`]); this
//! draws the part of the plan the canvas shows, on a worker thread, each time the view, the
//! settings, the position or the plan change. Until the new picture arrives the one before it
//! stays, moved and scaled to where its part of the document now is.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Color32, Rect, TextureHandle, Ui, pos2};
use vectorcraft_engine::stitch_preview::{FABRIC, Frame, MAX_REGION_SIDE, MM_PER_PT, REGION_SCALES, RegionOptions};
use vectorcraft_geom::Point;

use super::{Xf, textured_quad};
use crate::VectorcraftApp;
use crate::panels::stitch_preview::{self, OnCanvas};

/// The fabric the artboards are drawn in.
pub(super) const FABRIC_COLOR: Color32 = Color32::from_rgb(FABRIC[0], FABRIC[1], FABRIC[2]);
/// How much of the fabric colour covers the art shown as an underlay.
const UNDERLAY_WASH: u8 = 200;

/// A picture asked for or shown: of which plan, and of what part of the document.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    uid: u64,
    generation: u64,
    region: RegionOptions,
}

#[derive(Default)]
struct Runtime {
    shown: Option<(Key, TextureHandle)>,
    job: Option<(Key, Receiver<Result<Frame, String>>)>,
    /// Why the picture asked for last could not be drawn.
    error: Option<(Key, String)>,
}

thread_local! {
    static RT: RefCell<Runtime> = RefCell::new(Runtime::default());
}

/// The document rectangle the canvas shows (the bounding one when the view is turned) and the
/// picture of it to ask for: pixels per millimetre within what a region takes, at most
/// [`MAX_REGION_SIDE`] pixels a side. `None` when the view is not a finite one.
fn region_of(xf: &Xf, rect: Rect, ppp: f32) -> Option<(f64, f64, f64, u32, u32)> {
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].map(|c| xf.to_doc(c));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for c in corners {
        (x0, y0, x1, y1) = (x0.min(c.x), y0.min(c.y), x1.max(c.x), y1.max(c.y));
    }
    let (w_mm, h_mm) = ((x1 - x0) * MM_PER_PT, (y1 - y0) * MM_PER_PT);
    if !(x0.is_finite() && y0.is_finite() && w_mm.is_finite() && h_mm.is_finite() && w_mm > 0.0 && h_mm > 0.0) {
        return None;
    }
    // Screen pixels per millimetre, then within the renderer's range and the side cap: beyond
    // either, the picture is drawn coarser or finer than the screen and scaled.
    let screen = xf.zoom * f64::from(ppp) / MM_PER_PT;
    let cap = f64::from(MAX_REGION_SIDE) / w_mm.max(h_mm);
    let scale = screen.min(cap).clamp(*REGION_SCALES.start(), *REGION_SCALES.end());
    let side = |mm: f64| {
        let px = (mm * scale).ceil().clamp(1.0, f64::from(MAX_REGION_SIDE));
        // Within 1..=MAX_REGION_SIDE, so the cast is exact.
        px as u32
    };
    Some((x0 * MM_PER_PT, y0 * MM_PER_PT, scale, side(w_mm), side(h_mm)))
}

/// The screen corners of the document rectangle a picture with `r` covers.
fn quad_of(xf: &Xf, r: &RegionOptions) -> Vec<egui::Pos2> {
    let (x0, y0) = (r.left_mm / MM_PER_PT, r.top_mm / MM_PER_PT);
    let (x1, y1) = (x0 + f64::from(r.width) / r.scale / MM_PER_PT, y0 + f64::from(r.height) / r.scale / MM_PER_PT);
    [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)].iter().map(|p| xf.to_screen(*p)).collect()
}

/// For a view in the stitch view: the plan and the playback, advanced to now. `None` in Preview
/// and Outline.
pub(super) fn begin(app: &VectorcraftApp, ctx: &egui::Context, view: &crate::state::View) -> Option<OnCanvas> {
    view.stitches.then(|| stitch_preview::for_canvas(app, ctx))
}

/// Whether the canvas draws the art: always outside the stitch view (`on` is `None`); in it, when
/// asked for (Artwork underlay) or when there are no stitches to show in its place.
pub(super) fn shows_art(app: &VectorcraftApp, on: Option<&OnCanvas>) -> bool {
    on.is_none_or(|on| app.ui.stitch_preview.underlay || !matches!(on.plan, Some(Ok(_))))
}

/// The stitch view's layer over the canvas `rect`: the fabric wash over art that shows, the
/// stitches, and a note when there are none. Nothing outside the stitch view.
pub(super) fn draw(
    app: &VectorcraftApp,
    ui: &Ui,
    (painter, xf, rect): (&egui::Painter, &Xf, Rect),
    doc: &vectorcraft_doc::Document,
    on: Option<&OnCanvas>,
) {
    let Some(on) = on else { return };
    if shows_art(app, Some(on)) {
        wash(painter, xf, doc);
    }
    if let Some(why) = paint(app, ui, painter, xf, rect, on) {
        note(painter, rect, &why, &crate::theme::Tokens::get(ui.ctx()));
    }
}

/// The fabric colour over the artboards, thin enough to see the art through: drawn over the art
/// when it shows as an underlay.
fn wash(painter: &egui::Painter, xf: &Xf, doc: &vectorcraft_doc::Document) {
    let [r, g, b] = FABRIC;
    for ab in &doc.artboards {
        painter.add(egui::Shape::convex_polygon(xf.quad(ab.rect), Color32::from_rgba_unmultiplied(r, g, b, UNDERLAY_WASH), egui::Stroke::NONE));
    }
}

/// Draw the stitches of the active document over the canvas `rect`, and ask for a newer picture
/// when the one shown is not of this view. → a line to show when there are no stitches (why).
fn paint(app: &VectorcraftApp, ui: &Ui, painter: &egui::Painter, xf: &Xf, rect: Rect, on: &OnCanvas) -> Option<String> {
    let uid = app.session.active()?.uid;
    let (generation, plan) = match &on.plan {
        Some(Ok(p)) => p.clone(),
        Some(Err(e)) => {
            RT.with(|rt| *rt.borrow_mut() = Runtime::default());
            return Some(e.clone());
        }
        // The first plan is being built.
        None => return None,
    };
    let ctx = ui.ctx();
    let want = region_of(xf, rect, ctx.pixels_per_point()).map(|(left_mm, top_mm, scale, width, height)| Key {
        uid,
        generation,
        region: RegionOptions { frame: stitch_preview::frame_options(&app.ui.stitch_preview, on.position), left_mm, top_mm, scale, width, height },
    });
    RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        // A finished picture.
        if let Some((k, rx)) = rt.job.take() {
            match rx.try_recv() {
                Ok(Ok(f)) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                    rt.shown = Some((k, ctx.load_texture("canvas-stitches", image, egui::TextureOptions::LINEAR)));
                    rt.error = None;
                }
                Ok(Err(e)) => rt.error = Some((k, e)),
                Err(TryRecvError::Empty) => rt.job = Some((k, rx)),
                Err(TryRecvError::Disconnected) => rt.error = Some((k, "the stitch view stopped unexpectedly".into())),
            }
        }
        // Another document's picture is not this one's.
        if rt.shown.as_ref().is_some_and(|(k, _)| k.uid != uid) {
            rt.shown = None;
        }
        if let Some(want) = want {
            let have = rt.shown.as_ref().map(|(k, _)| *k) == Some(want) || rt.error.as_ref().map(|(k, _)| *k) == Some(want);
            if !have && rt.job.is_none() {
                let region = want.region;
                rt.job = Some((want, stitch_preview::spawn(ctx, move || plan.region(&region))));
            }
        }
        if let Some((k, tex)) = &rt.shown {
            textured_quad(painter, tex.id(), &quad_of(xf, &k.region), (1.0, 1.0));
        }
        rt.error.as_ref().filter(|(k, _)| Some(*k) == want).map(|(_, e)| e.clone())
    })
}

/// A line of text in the middle of the canvas's top: why there are no stitches to show.
fn note(painter: &egui::Painter, rect: Rect, text: &str, t: &crate::theme::Tokens) {
    let galley = painter.layout(text.to_owned(), egui::FontId::proportional(13.0), t.text, (rect.width() - 48.0).max(120.0));
    let at = pos2(rect.center().x - galley.size().x / 2.0, rect.top() + 16.0);
    let back = Rect::from_min_size(at, galley.size()).expand2(egui::vec2(10.0, 6.0));
    painter.rect_filled(back, 6.0, t.panel);
    painter.galley(at, galley, t.text);
}

/// What the stitch view is showing, for `ui.inspect` and tests: the picture's size in pixels and
/// its scale in pixels per millimetre, or nothing yet.
pub(crate) fn state() -> serde_json::Value {
    RT.with(|rt| {
        let rt = rt.borrow();
        match &rt.shown {
            Some((k, _)) => serde_json::json!({
                "width": k.region.width,
                "height": k.region.height,
                "scale": k.region.scale,
                "leftMm": k.region.left_mm,
                "topMm": k.region.top_mm,
                "stitches": k.region.frame.stitches,
                "pending": rt.job.is_some(),
            }),
            None => serde_json::json!({ "pending": rt.job.is_some() }),
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::Session;

    use super::*;
    use crate::panels::stitch_preview;

    fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
        app.run(id, p).unwrap()
    }

    fn app_with_art() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 600, "height": 400}));
        run(&mut app, "paint.setFill", json!({"none": true}));
        run(&mut app, "paint.setStroke", json!({"color": "#ff0000"}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50}));
        run(&mut app, "select.none", json!({}));
        app
    }

    /// Draw the canvas, 800 × 600 points, frame after frame until `until` holds for the stitch
    /// view's state: the texts and the number of textured meshes of the last frame.
    fn frames(app: &mut VectorcraftApp, until: impl Fn(&Value) -> bool) -> (Vec<String>, usize) {
        fn walk(s: &egui::Shape, texts: &mut Vec<String>, meshes: &mut usize) {
            match s {
                egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
                egui::Shape::Mesh(m) if matches!(m.texture_id, egui::TextureId::Managed(id) if id != 0) => *meshes += 1,
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, texts, meshes)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        for i in 0..400 {
            let input = egui::RawInput {
                time: Some(f64::from(i) * 0.05),
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| super::super::show(app, ui));
            out.textures_delta.clear();
            let (mut texts, mut meshes) = (vec![], 0);
            out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts, &mut meshes));
            if until(&state()) {
                return (texts, meshes);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the stitch view did not settle: {}", state());
    }

    #[test]
    fn the_stitch_view_and_outline_are_one_or_the_other() {
        let mut app = app_with_art();
        assert!(!app.stitch_view());
        assert_eq!(run(&mut app, "view.stitches", json!({})), json!(true));
        assert!(app.stitch_view() && crate::menus::checked(&app, "view.stitches", &json!({})) == Some(true));
        // Outline takes over, and the stitch view takes over from it.
        run(&mut app, "view.outline", json!({}));
        assert!(app.outline_view() && !app.stitch_view());
        assert_eq!(run(&mut app, "view.stitches", json!({"on": true})), json!(true));
        assert!(app.stitch_view() && !app.outline_view());
        let title = crate::chrome::document_window_title(&app, crate::doc_windows::tab_window(&app).unwrap()).unwrap();
        assert!(title.ends_with("(RGB/Stitches)"), "{title}");
        // Setting it to what it is changes nothing; off leaves Preview.
        assert_eq!(run(&mut app, "view.stitches", json!({"on": true})), json!(true));
        assert_eq!(run(&mut app, "view.stitches", json!({"on": false})), json!(false));
        assert!(!app.stitch_view() && !app.outline_view());
        // Without a document there is no view to set.
        let mut none = VectorcraftApp::new(Session::new(), Default::default());
        assert!(none.run("view.stitches", json!({})).is_err());
        assert!(!crate::menus::enabled(&none, "view.stitches"));
    }

    #[test]
    fn an_embroidery_app_opens_documents_in_the_stitch_view() {
        let mut app = app_with_art();
        app.open_in_stitch_view();
        assert!(app.stitch_view(), "the document already open");
        run(&mut app, "file.new", json!({"width": 300, "height": 300}));
        assert!(app.stitch_view(), "a new document");
        // It is still each document's own setting.
        run(&mut app, "view.stitches", json!({"on": false}));
        assert!(!app.stitch_view());
    }

    #[test]
    fn the_canvas_shows_the_stitches_at_its_own_zoom_and_follows_the_playback() {
        let mut app = app_with_art();
        // In Preview nothing of the stitch view is drawn or asked for.
        let (_, before) = frames(&mut app, |_| true);
        assert_eq!(state(), json!({"pending": false}));
        run(&mut app, "view.stitches", json!({}));
        let (texts, meshes) = frames(&mut app, |s| s["width"].is_u64() && s["pending"] == false);
        assert!(meshes > 0 && meshes >= before, "the stitches are a textured quad: {meshes} after {before}");
        assert!(!texts.iter().any(|t| t.contains("no ")), "no note when there are stitches: {texts:?}");
        // One picture pixel per screen pixel: the canvas's zoom in pixels per millimetre.
        let (s, v) = (state(), *app.view().unwrap());
        let ppp = 1.0;
        assert!((s["scale"].as_f64().unwrap() - v.zoom * ppp / MM_PER_PT).abs() < 1e-9, "{s} at zoom {}", v.zoom);
        assert!(s["width"].as_u64().unwrap() <= 801 && s["height"].as_u64().unwrap() <= 601, "{s}");
        assert_eq!(s["stitches"], Value::Null);
        // The playback position of the panel is the canvas's.
        stitch_preview::command(&mut app, &json!({"position": 5})).unwrap();
        frames(&mut app, |s| s["stitches"] == 5 && s["pending"] == false);
        // A zoom asks for a picture at the new scale.
        run(&mut app, "view.zoomIn", json!({}));
        let zoom = app.view().unwrap().zoom;
        frames(&mut app, |s| {
            s["scale"].as_f64().is_some_and(|x| (x - (zoom / MM_PER_PT).min(*REGION_SCALES.end())).abs() < 1e-9) && s["pending"] == false
        });
        let panel = stitch_preview::state(&app);
        assert_eq!(panel["onCanvas"], true);
        assert!(panel["canvas"]["width"].is_u64(), "{panel}");
    }

    #[test]
    fn a_document_with_nothing_to_sew_says_why_and_keeps_its_art() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 600, "height": 400}));
        run(&mut app, "view.stitches", json!({}));
        let (texts, _) = frames(&mut app, |_| stitch_preview::state_message().is_some());
        let why = stitch_preview::state_message().unwrap();
        assert!(texts.contains(&why), "{why:?} in {texts:?}");
        assert_eq!(state(), json!({"pending": false}), "nothing is drawn of a plan that is not there");
    }

    #[test]
    fn the_region_fits_the_side_cap_and_the_scale_range() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let at = |zoom: f64, rot: f64| region_of(&Xf { rect, zoom, center: Point::new(300.0, 200.0), rot }, rect, 2.0);
        // 100%: 2 pixels per point, 1600 × 1200 pixels, from the canvas's top-left document point.
        let (left, top, scale, w, h) = at(1.0, 0.0).unwrap();
        assert!((scale - 2.0 / MM_PER_PT).abs() < 1e-9 && (w, h) == (1600, 1200), "{scale} {w} {h}");
        assert!((left - (300.0 - 400.0) * MM_PER_PT).abs() < 1e-9 && (top - (200.0 - 300.0) * MM_PER_PT).abs() < 1e-9);
        // Far in: no finer than the renderer draws. Far out: no coarser.
        assert_eq!(at(640.0, 0.0).unwrap().2, *REGION_SCALES.end());
        assert_eq!(at(0.001, 0.0).unwrap().2, *REGION_SCALES.start());
        // Turned: the bounding rectangle, still within the cap.
        let (_, _, _, w, h) = at(1.0, 0.7).unwrap();
        assert!(w > 1600 && h > 1200 && w <= MAX_REGION_SIDE && h <= MAX_REGION_SIDE);
        // A view that is not a number draws nothing rather than panicking.
        assert!(at(f64::NAN, 0.0).is_none());
        let q = quad_of(
            &Xf { rect, zoom: 1.0, center: Point::new(300.0, 200.0), rot: 0.0 },
            &RegionOptions { frame: Default::default(), left_mm: left, top_mm: top, scale, width: 1600, height: 1200 },
        );
        assert!((q[0].x - 0.0).abs() < 0.01 && (q[2].x - 800.0).abs() < 0.01 && (q[2].y - 600.0).abs() < 0.01, "{q:?}");
    }
}
