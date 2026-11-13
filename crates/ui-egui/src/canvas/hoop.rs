//! View › Hoop: a design's embroidery hoop on the canvas, in every view. The hoop's inner edge in
//! its shape (a rectangle with slightly rounded corners, or an ellipse) and a band outside it for
//! the frame, centred where `stitch.hoop.center` centres the design (the first artboard's centre);
//! the sewing field, [`HOOP_MARGIN_MM`] inside the edge, as a dashed line; the canvas outside the
//! field dimmed; and the hoop's name over its top-left corner. Nothing for a design without a hoop.

use egui::{Color32, Pos2, Shape, Stroke};
use vectorcraft_doc::Document;
use vectorcraft_engine::stitch_preview::{HOOP_MARGIN_MM, Hoop, HoopShape, MM_PER_PT, hoop_center};
use vectorcraft_geom::Point;

use super::Xf;
use crate::VectorcraftApp;
use crate::theme::Tokens;

/// Points along each rounded corner of a rectangular hoop, and around a round one.
const CORNER_STEPS: usize = 8;
const ROUND_STEPS: usize = 96;
/// The rounding of a rectangular hoop's inner corners, mm (less on a small hoop).
const CORNER_MM: f64 = 6.0;
/// How wide the frame band outside the inner edge is drawn, mm.
const BAND_MM: f64 = 7.0;

/// The outline of `hoop` centred on `c` (document points), `inset_mm` inside its inner edge
/// (negative: outside it), going round once. A rectangle's corners are rounded by `corner_mm`
/// more than the inset (0: square). Every outline of one shape has the same number of points, so
/// two of them make a band. Empty when the inset leaves nothing or a size is not finite.
pub(crate) fn outline(hoop: &Hoop, c: Point, inset_mm: f64, corner_mm: f64) -> Vec<Point> {
    let pt = 1.0 / MM_PER_PT;
    let (hx, hy) = ((hoop.width_mm / 2.0 - inset_mm) * pt, (hoop.height_mm / 2.0 - inset_mm) * pt);
    if !(hx.is_finite() && hy.is_finite() && hx > 0.0 && hy > 0.0 && c.x.is_finite() && c.y.is_finite()) {
        return vec![];
    }
    match hoop.shape {
        HoopShape::Round => (0..ROUND_STEPS)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / ROUND_STEPS as f64;
                Point::new(c.x + hx * a.cos(), c.y + hy * a.sin())
            })
            .collect(),
        HoopShape::Rectangle => {
            let r = (corner_mm * pt).clamp(0.0, hx.min(hy));
            // Each corner's arc centre and the angle its arc starts at, clockwise from the top right.
            let corners = [((hx - r, -(hy - r)), -90.0f64), ((hx - r, hy - r), 0.0), ((-(hx - r), hy - r), 90.0), ((-(hx - r), -(hy - r)), 180.0)];
            corners
                .iter()
                .flat_map(|&((ax, ay), start)| {
                    (0..=CORNER_STEPS).map(move |k| {
                        let a = (start + 90.0 * k as f64 / CORNER_STEPS as f64).to_radians();
                        Point::new(c.x + ax + r * a.cos(), c.y + ay + r * a.sin())
                    })
                })
                .collect()
        }
    }
}

/// The hoop of `doc`'s design, if View › Hoop shows it.
fn shown(app: &VectorcraftApp, doc: &Document) -> Option<Hoop> {
    app.ui.view.hoop.then(|| doc.stitch.hoop().cloned()).flatten()
}

/// Draw the hoop of `doc` over the canvas `rect` (see the module).
pub(super) fn draw(app: &VectorcraftApp, painter: &egui::Painter, xf: &Xf, rect: egui::Rect, doc: &Document, t: &Tokens) {
    let Some(hoop) = shown(app, doc) else { return };
    let c = hoop_center(doc);
    let screen = |pts: Vec<Point>| -> Vec<Pos2> { pts.into_iter().map(|p| xf.to_screen(p)).collect() };
    let edge = screen(outline(&hoop, c, 0.0, CORNER_MM));
    if edge.len() < 3 {
        return;
    }
    let field = screen(outline(&hoop, c, HOOP_MARGIN_MM, 0.0));
    let centre = xf.to_screen(c);
    // Outside the sewing field (the edge when the margin leaves none), dimmed.
    dim_outside(painter, rect, centre, if field.len() >= 3 { &field } else { &edge }, t.hoop_outside);
    // The frame: a band from the inner edge out.
    let outer = screen(outline(&hoop, c, -BAND_MM, CORNER_MM + BAND_MM));
    painter.add(band(&edge, &outer, t.hoop.gamma_multiply(0.35)));
    painter.add(Shape::closed_line(edge.clone(), Stroke::new(1.5, t.hoop)));
    if field.len() >= 3 {
        let mut ring = field;
        if let Some(first) = ring.first().copied() {
            ring.push(first);
        }
        painter.extend(Shape::dashed_line(&ring, Stroke::new(1.0, t.hoop_field), 6.0, 4.0));
    }
    // The name centred over the frame, where it reads on the pasteboard; kept on the canvas.
    if let Some(top) = outer.iter().copied().reduce(|a, b| if a.y < b.y { a } else { b }) {
        let at = egui::pos2(centre.x, (top.y - 4.0).max(rect.top() + 16.0));
        painter.text(at, egui::Align2::CENTER_BOTTOM, &hoop.name, egui::FontId::proportional(11.0), t.text_dim);
    }
}

/// The ring between two outlines of one shape (`inner` and `outer`, the same number of points), in
/// `fill`.
fn band(inner: &[Pos2], outer: &[Pos2], fill: Color32) -> Shape {
    let n = inner.len().min(outer.len());
    let mut mesh = egui::Mesh::default();
    for (a, b) in inner.iter().zip(outer).take(n) {
        mesh.colored_vertex(*a, fill);
        mesh.colored_vertex(*b, fill);
    }
    // Within u32: an outline has at most a few hundred points.
    let n = u32::try_from(n).unwrap_or(0);
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.add_triangle(2 * i, 2 * i + 1, 2 * j + 1);
        mesh.add_triangle(2 * i, 2 * j + 1, 2 * j);
    }
    Shape::mesh(mesh)
}

/// Dim the canvas `rect` outside the convex outline `inner` around `centre`: a band from it out to
/// a copy scaled about the centre past the canvas's corners.
fn dim_outside(painter: &egui::Painter, rect: egui::Rect, centre: Pos2, inner: &[Pos2], fill: Color32) {
    let near = inner.iter().map(|p| p.distance(centre)).fold(f32::INFINITY, f32::min);
    let far = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].iter().map(|p| p.distance(centre)).fold(0.0, f32::max);
    // Too small to see from the centre, or not a finite view: no dimming.
    if !(near.is_finite() && far.is_finite() && near > 0.5) {
        return;
    }
    let k = far / near + 1.0;
    let outer: Vec<Pos2> = inner.iter().map(|p| centre + (*p - centre) * k).collect();
    painter.add(band(inner, &outer, fill));
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::stitch_preview::HoopShape;

    use super::*;

    fn hoop(shape: HoopShape) -> Hoop {
        Hoop { id: None, name: "h".into(), width_mm: 130.0, height_mm: 180.0, shape, cap: false }
    }

    #[test]
    fn the_hoop_path_is_made_for_both_shapes() {
        let pt = 1.0 / MM_PER_PT;
        let c = Point::new(500.0, 400.0);
        for shape in [HoopShape::Rectangle, HoopShape::Round] {
            let edge = outline(&hoop(shape), c, 0.0, CORNER_MM);
            let field = outline(&hoop(shape), c, 10.0, 0.0);
            let outer = outline(&hoop(shape), c, -BAND_MM, CORNER_MM + BAND_MM);
            assert!(edge.len() >= 32 && edge.len() == field.len() && edge.len() == outer.len(), "{shape:?}: {}", edge.len());
            let (x0, x1) = edge.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
            let (y0, y1) = edge.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y), b.max(p.y)));
            // As wide and tall as the hoop, about its centre.
            assert!((x1 - x0 - 130.0 * pt).abs() < 1e-6 && (y1 - y0 - 180.0 * pt).abs() < 1e-6, "{shape:?}");
            assert!(((x0 + x1) / 2.0 - c.x).abs() < 1e-6 && ((y0 + y1) / 2.0 - c.y).abs() < 1e-6, "{shape:?}");
            // The field is 10 mm in on every side.
            let fx = field.iter().fold(f64::NEG_INFINITY, |a, p| a.max(p.x));
            assert!((fx - (c.x + 55.0 * pt)).abs() < 1e-6, "{shape:?}");
        }
        // A rectangle's corner is rounded: its corner point is not on the outline.
        let rect = outline(&hoop(HoopShape::Rectangle), c, 0.0, CORNER_MM);
        let corner = Point::new(c.x + 65.0 * pt, c.y - 90.0 * pt);
        assert!(rect.iter().all(|p| (p.x - corner.x).hypot(p.y - corner.y) > 1.0));
        // The field's corner is square.
        let field = outline(&hoop(HoopShape::Rectangle), c, 10.0, 0.0);
        assert!(field.iter().any(|p| (p.x - (c.x + 55.0 * pt)).abs() < 1e-9 && (p.y - (c.y - 80.0 * pt)).abs() < 1e-9));
        // A margin past the middle, or a size that is not a number, leaves nothing.
        assert!(outline(&hoop(HoopShape::Round), c, 100.0, 0.0).is_empty());
        assert!(outline(&Hoop { width_mm: f64::NAN, ..hoop(HoopShape::Rectangle) }, c, 0.0, CORNER_MM).is_empty());
    }

    /// The texts of one canvas frame, 800 × 600 points.
    fn canvas_texts(app: &mut VectorcraftApp) -> Vec<String> {
        fn walk(s: &Shape, out: &mut Vec<String>) {
            match s {
                Shape::Text(t) => out.push(t.galley.text().to_string()),
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| super::super::show(app, ui));
        out.textures_delta.clear();
        let mut texts = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
        texts
    }

    #[test]
    fn view_hoop_draws_the_designs_hoop_in_every_view_and_toggles() {
        use serde_json::json;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"hoop": "round-150"})).unwrap();
        assert_eq!(crate::menus::checked(&app, "view.hoop", &json!({})), Some(true), "on by default");
        assert!(canvas_texts(&mut app).iter().any(|t| t == "Round 150 mm"), "Preview");
        app.run("view.outline", json!({})).unwrap();
        assert!(canvas_texts(&mut app).iter().any(|t| t == "Round 150 mm"), "Outline");
        app.run("view.stitches", json!({"on": true})).unwrap();
        assert!(canvas_texts(&mut app).iter().any(|t| t == "Round 150 mm"), "Stitches");
        assert_eq!(app.run("view.hoop", json!({})).unwrap(), json!(false));
        assert!(!canvas_texts(&mut app).iter().any(|t| t == "Round 150 mm"));
        assert_eq!(app.run("view.hoop", json!({"on": true})).unwrap(), json!(true));
        // Kept with the other view toggles, and on in state saved without it.
        let saved = serde_json::to_value(&app.ui.view).unwrap();
        assert_eq!(saved["hoop"], true);
        let mut old = saved.clone();
        old.as_object_mut().unwrap().remove("hoop");
        assert!(serde_json::from_value::<crate::state::ViewFlags>(old).unwrap().hoop);
        // A design without a hoop draws none.
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        assert!(!canvas_texts(&mut app).iter().any(|t| t.contains("mm")));
    }

    #[test]
    fn the_band_and_the_dimming_close_their_rings() {
        let inner: Vec<Pos2> = (0..8).map(|i| Pos2::new(100.0 + 10.0 * (i as f32).cos(), 100.0 + 10.0 * (i as f32).sin())).collect();
        let outer: Vec<Pos2> = inner.iter().map(|p| Pos2::new(100.0, 100.0) + (*p - Pos2::new(100.0, 100.0)) * 2.0).collect();
        let Shape::Mesh(m) = band(&inner, &outer, Color32::RED) else { panic!("a mesh") };
        assert_eq!((m.vertices.len(), m.indices.len()), (16, 8 * 6));
        assert!(m.indices.iter().all(|i| (*i as usize) < 16));
    }
}
