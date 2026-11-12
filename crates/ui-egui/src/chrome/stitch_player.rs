//! The stitch player: a bar under the canvas while it shows the stitches (View › Stitches), with
//! the playback of the plan the canvas draws. To the start, back and forward a colour block, play
//! or pause, to the end; the speed; a scrubber over the whole design with a segment per thread
//! block in its colour and a tick at each trim and stop; what is shown; and where a playback
//! pauses by itself. The playback itself is [`crate::panels::stitch_preview`]'s: the bar, the
//! Stitch Preview panel and the canvas show one position.

use egui::{Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use vectorcraft_engine::stitch_preview::{MM_PER_PT, Mark};
use vectorcraft_geom::Point;

use super::{Tokens, VectorcraftApp};
use crate::panels::stitch_preview::{self, Player};

/// The bar's height in points.
const HEIGHT: f32 = 34.0;
/// How near the canvas's edge the needle may come before the view follows it, in points.
const FOLLOW_MARGIN: f32 = 48.0;

/// A transport button's picture.
#[derive(Clone, Copy)]
enum Glyph {
    Start,
    Back,
    Play,
    Pause,
    Forward,
    End,
}

/// A 24-point button drawn as `glyph`, from triangles and bars. → clicked.
fn transport(ui: &mut Ui, t: &Tokens, glyph: Glyph, tip: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let resp = resp.on_hover_text(tip);
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let color = if !enabled {
        t.text_dim
    } else if matches!(glyph, Glyph::Play | Glyph::Pause) {
        t.accent
    } else {
        t.text
    };
    let (c, p) = (rect.center(), ui.painter());
    // A triangle pointing right (`dir` 1) or left (-1) with its flat side at `x`, and a bar at `x`.
    let tri = |x: f32, dir: f32, half: f32| {
        p.add(egui::Shape::convex_polygon(vec![pos2(x, c.y - half), pos2(x + dir * half * 1.5, c.y), pos2(x, c.y + half)], color, Stroke::NONE));
    };
    let bar = |x: f32, half: f32| p.rect_filled(Rect::from_min_max(pos2(x - 1.0, c.y - half), pos2(x + 1.0, c.y + half)), 0.0, color);
    match glyph {
        Glyph::Play => tri(c.x - 4.5, 1.0, 6.5),
        Glyph::Pause => {
            p.rect_filled(Rect::from_min_max(pos2(c.x - 5.0, c.y - 6.0), pos2(c.x - 1.5, c.y + 6.0)), 0.0, color);
            p.rect_filled(Rect::from_min_max(pos2(c.x + 1.5, c.y - 6.0), pos2(c.x + 5.0, c.y + 6.0)), 0.0, color);
        }
        Glyph::Start => {
            bar(c.x - 5.0, 5.0);
            tri(c.x + 5.0, -1.0, 5.0);
        }
        Glyph::End => {
            tri(c.x - 5.0, 1.0, 5.0);
            bar(c.x + 5.0, 5.0);
        }
        Glyph::Back => {
            tri(c.x, -1.0, 4.5);
            tri(c.x + 7.0, -1.0, 4.5);
        }
        Glyph::Forward => {
            tri(c.x - 7.0, 1.0, 4.5);
            tri(c.x, 1.0, 4.5);
        }
    }
    enabled && resp.clicked()
}

/// The stitch count at horizontal position `x` of a scrubber `rect` over `total` stitches.
fn stitch_at(rect: Rect, x: f32, total: usize) -> usize {
    if rect.width() <= 0.0 || total == 0 {
        return 0;
    }
    let f = f64::from(((x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    // Within 0..=total, so the cast is exact.
    (f * total as f64).round() as usize
}

/// The scrubber: the design from its first stitch to its last. → the stitch count to move to.
fn scrubber(ui: &mut Ui, t: &Tokens, p: &Player, width: f32) -> Option<usize> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(40.0), 18.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(2.0));
    let track = rect.shrink2(vec2(0.0, 2.0));
    let total = p.total.max(1) as f32;
    let x_of = |n: usize| track.left() + track.width() * (n as f32 / total).clamp(0.0, 1.0);
    painter.rect_filled(track, 3.0, t.input);
    for b in p.blocks.iter().filter(|b| b.stitches > 0) {
        let [r, g, bl] = b.rgb;
        let seg = Rect::from_min_max(pos2(x_of(b.first), track.top()), pos2(x_of(b.first + b.stitches), track.bottom()));
        painter.rect_filled(seg, 0.0, Color32::from_rgb(r, g, bl));
        // A hairline where the colour changes, so two blocks of one colour read as two.
        painter.vline(seg.left(), track.y_range(), Stroke::new(1.0, t.panel));
    }
    let at = p.position.unwrap_or(p.total);
    // What is not sewn yet is dimmed.
    let ahead = Rect::from_min_max(pos2(x_of(at), track.top()), track.right_bottom());
    painter.rect_filled(ahead, 0.0, t.panel.gamma_multiply(0.62));
    for (n, mark) in &p.marks {
        let (color, len) = if *mark == Mark::Stop { (t.warning, track.height()) } else { (t.text_strong, 5.0) };
        painter.vline(x_of(*n), track.top()..=track.top() + len, Stroke::new(1.0, color));
    }
    painter.rect_stroke(track, 3.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let head = x_of(at);
    painter.vline(head, rect.y_range(), Stroke::new(2.0, t.text_strong));
    if resp.has_focus() {
        painter.rect_stroke(rect.expand(1.0), 4.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Outside);
    }
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    let mut to = None;
    if (resp.clicked() || resp.dragged())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        to = Some(stitch_at(track, pos.x, p.total));
    }
    if resp.has_focus() {
        let (left, right, shift) = ui.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.modifiers.shift));
        let by = if shift { 10 } else { 1 };
        if left || right {
            stitch_preview::nudge(if left { -by } else { by });
        }
    }
    if let Some(pos) = resp.hover_pos().filter(|_| !resp.dragged()) {
        let n = stitch_at(track, pos.x, p.total);
        resp.on_hover_text(format!("{} {n} / {}", tl!("Stitch"), p.total));
    }
    to
}

/// The Pause At menu: where a playback stops by itself, and whether the canvas follows the needle.
fn pause_menu(ui: &mut Ui, set: &mut stitch_preview::Settings) {
    ui.menu_button(tl!("Pause At"), |ui| {
        ui.checkbox(&mut set.pause_at.color_changes, tl!("Colour Changes"));
        ui.checkbox(&mut set.pause_at.trims, tl!("Trims"));
        ui.checkbox(&mut set.pause_at.stops, tl!("Stops"));
        ui.separator();
        ui.checkbox(&mut set.follow, tl!("Follow the Needle")).on_hover_text(tl!("During a playback, move the canvas to keep the needle in sight"));
    });
}

/// During a playback, with Follow the Needle on: move the view so the needle is back in the
/// middle once it has come within [`FOLLOW_MARGIN`] of the canvas's edge or left it.
fn follow(app: &mut VectorcraftApp, p: &Player) {
    if !(p.playing && app.ui.stitch_preview.follow) {
        return;
    }
    let (Some((x, y)), Some(rect)) = (p.needle_mm, crate::canvas::stitch_view_rect()) else { return };
    let needle = Point::new(x / MM_PER_PT, y / MM_PER_PT);
    let Some(view) = app.view_mut() else { return };
    let at = crate::canvas::Xf::new(rect, view).to_screen(needle);
    if !rect.shrink(FOLLOW_MARGIN.min(rect.width() / 4.0).min(rect.height() / 4.0)).contains(at) && needle.x.is_finite() && needle.y.is_finite() {
        view.center = needle;
    }
}

/// What the bar says of the position: "1,204 / 8,240" and the colour block being sewn.
fn readout(p: &Player) -> String {
    let at = p.position.unwrap_or(p.total);
    let sewn = |b: &&vectorcraft_engine::stitch_preview::BlockInfo| b.stitches > 0;
    // The blocks started so far: the last of them is the one being sewn.
    let block = p.blocks.iter().filter(sewn).filter(|b| b.first < at).count();
    let blocks = p.blocks.iter().filter(sewn).count();
    if block > 0 { format!("{at} / {} · {} {block}/{blocks}", p.total, tl!("colour")) } else { format!("{at} / {}", p.total) }
}

/// The stitch player bar, while the active document is in the stitch view.
pub fn stitch_player(app: &mut VectorcraftApp, ui: &mut Ui) {
    if !app.stitch_view() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let player = stitch_preview::player();
    if let Some(p) = &player {
        follow(app, p);
    }
    let mut set = app.ui.stitch_preview.clone();
    egui::Panel::bottom("stitch_player")
        .exact_size(HEIGHT)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(8, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let Some(p) = &player else {
                    ui.label(egui::RichText::new(tl!("No stitches to play yet")).color(t.text_dim).size(12.0));
                    return;
                };
                let any = p.total > 0;
                let at = p.position.unwrap_or(p.total);
                if transport(ui, &t, Glyph::Start, tl!("To the first stitch"), any && at > 0) {
                    stitch_preview::seek(Some(0));
                }
                if transport(ui, &t, Glyph::Back, tl!("Back one colour block"), any && at > 0) {
                    stitch_preview::step_block(false);
                }
                let (glyph, tip) = if p.playing { (Glyph::Pause, tl!("Pause")) } else { (Glyph::Play, tl!("Play the stitches in sewing order")) };
                if transport(ui, &t, glyph, tip, any) {
                    stitch_preview::toggle_play();
                }
                if transport(ui, &t, Glyph::Forward, tl!("Forward one colour block"), any && at < p.total) {
                    stitch_preview::step_block(true);
                }
                if transport(ui, &t, Glyph::End, tl!("To the last stitch"), any && at < p.total) {
                    stitch_preview::seek(None);
                }
                ui.add_space(6.0);
                stitch_preview::speed_menu(ui, &mut set, "stitch-player-speed");
                ui.add_space(6.0);
                let text = readout(p);
                let room = ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), t.text).size().x;
                // The scrubber takes what the read-out and the menu leave.
                let width = ui.available_width() - room - 110.0;
                if let Some(to) = scrubber(ui, &t, p, width) {
                    stitch_preview::seek(Some(to));
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new(text).color(t.text).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| pause_menu(ui, &mut set));
            });
        });
    if set != app.ui.stitch_preview {
        app.ui.stitch_preview = set;
    }
    if player.is_some_and(|p| p.playing) {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::Session;

    use super::*;

    fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
        app.run(id, p).unwrap()
    }

    /// Two colour blocks: a red rectangle, then a blue ellipse.
    fn app_with_art() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 600, "height": 400}));
        run(&mut app, "paint.setFill", json!({"none": true}));
        run(&mut app, "paint.setStroke", json!({"color": "#ff0000"}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50}));
        run(&mut app, "select.none", json!({}));
        run(&mut app, "paint.setStroke", json!({"color": "#0000ff"}));
        run(&mut app, "shape.ellipse", json!({"x": 20, "y": 80, "width": 60, "height": 40}));
        run(&mut app, "select.none", json!({}));
        app
    }

    /// Draw the bar and the canvas, frame after frame at `step` seconds apart, until `until`
    /// holds: the texts of the last frame.
    fn frames(app: &mut VectorcraftApp, ctx: &egui::Context, clock: &mut f64, step: f64, until: impl Fn(&VectorcraftApp) -> bool) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        for _ in 0..600 {
            *clock += step;
            let input = egui::RawInput {
                time: Some(*clock),
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                stitch_player(app, ui);
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::show(app, ui));
            });
            out.textures_delta.clear();
            let mut last = vec![];
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut last));
            if until(app) {
                return last;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the player did not get there: {}", stitch_preview::state(app));
    }

    #[test]
    fn the_bar_shows_only_in_the_stitch_view_and_reads_the_position() {
        let mut app = app_with_art();
        let (ctx, mut clock) = (egui::Context::default(), 0.0);
        // In Preview there is no bar.
        let texts = frames(&mut app, &ctx, &mut clock, 0.05, |_| true);
        assert!(!texts.iter().any(|t| t == "Pause At"), "{texts:?}");
        run(&mut app, "view.stitches", json!({}));
        let texts = frames(&mut app, &ctx, &mut clock, 0.05, |_| stitch_preview::player().is_some());
        let texts = if texts.iter().any(|t| t == "Pause At") { texts } else { frames(&mut app, &ctx, &mut clock, 0.05, |_| true) };
        let p = stitch_preview::player().unwrap();
        assert!(p.total > 20 && p.blocks.len() == 2, "{} stitches", p.total);
        assert!(texts.iter().any(|t| t == "Pause At"), "{texts:?}");
        assert!(texts.iter().any(|t| *t == format!("{0} / {0} · colour 2/2", p.total)), "all stitches shown: {texts:?}");
        // The read-out follows the position; before the first stitch no block is being sewn.
        stitch_preview::seek(Some(0));
        let texts = frames(&mut app, &ctx, &mut clock, 0.05, |_| true);
        assert!(texts.iter().any(|t| *t == format!("0 / {}", p.total)), "{texts:?}");
        stitch_preview::seek(Some(3));
        let texts = frames(&mut app, &ctx, &mut clock, 0.05, |_| true);
        assert!(texts.iter().any(|t| *t == format!("3 / {} · colour 1/2", p.total)), "{texts:?}");
    }

    #[test]
    fn stepping_moves_by_colour_block_and_by_stitch_within_the_plan() {
        let mut app = app_with_art();
        let (ctx, mut clock) = (egui::Context::default(), 0.0);
        run(&mut app, "view.stitches", json!({}));
        frames(&mut app, &ctx, &mut clock, 0.05, |_| stitch_preview::player().is_some());
        let p = stitch_preview::player().unwrap();
        let second = p.blocks[1].first;
        let at = || stitch_preview::player().unwrap().position;
        // From the end: back to the start of the last block, then of the first, and no further.
        stitch_preview::step_block(false);
        assert_eq!(at(), Some(second));
        stitch_preview::step_block(false);
        assert_eq!(at(), Some(0));
        stitch_preview::step_block(false);
        assert_eq!(at(), Some(0));
        // Forward: the next block's start, then the end (all stitches).
        stitch_preview::step_block(true);
        assert_eq!(at(), Some(second));
        stitch_preview::step_block(true);
        assert_eq!(at(), None);
        // By stitches, clamped to the plan.
        stitch_preview::nudge(-10);
        assert_eq!(at(), Some(p.total - 10));
        stitch_preview::nudge(1_000_000);
        assert_eq!(at(), None);
        stitch_preview::seek(Some(2));
        stitch_preview::nudge(-5);
        assert_eq!(at(), Some(0));
        assert_eq!(stitch_at(Rect::from_min_size(pos2(100.0, 0.0), vec2(200.0, 10.0)), 200.0, 1000), 500);
        assert_eq!(stitch_at(Rect::from_min_size(pos2(100.0, 0.0), vec2(200.0, 10.0)), 5.0, 1000), 0);
        assert_eq!(stitch_at(Rect::from_min_size(pos2(100.0, 0.0), vec2(0.0, 10.0)), 150.0, 1000), 0);
    }

    #[test]
    fn a_playback_pauses_at_a_colour_change_when_asked_and_plays_on_from_there() {
        let mut app = app_with_art();
        let (ctx, mut clock) = (egui::Context::default(), 0.0);
        run(&mut app, "view.stitches", json!({}));
        frames(&mut app, &ctx, &mut clock, 0.05, |_| stitch_preview::player().is_some());
        let p = stitch_preview::player().unwrap();
        let second = p.blocks[1].first;
        let state =
            stitch_preview::command(&mut app, &json!({"pauseAt": {"colorChanges": true}, "speed": 400, "position": 0, "playing": true})).unwrap();
        assert_eq!(state["pauseAt"], json!({"colorChanges": true, "trims": false, "stops": false}));
        assert_eq!(state["follow"], true);
        frames(&mut app, &ctx, &mut clock, 0.05, |_| !stitch_preview::player().unwrap().playing);
        assert_eq!(stitch_preview::player().unwrap().position, Some(second), "stopped where the second colour starts");
        // Play again: it goes on to the end instead of stopping in place.
        stitch_preview::toggle_play();
        frames(&mut app, &ctx, &mut clock, 0.05, |_| !stitch_preview::player().unwrap().playing);
        assert_eq!(stitch_preview::player().unwrap().position, None);
        // Not asked: a playback runs through.
        stitch_preview::command(&mut app, &json!({"pauseAt": {"colorChanges": false}, "position": 0, "playing": true})).unwrap();
        frames(&mut app, &ctx, &mut clock, 0.05, |_| !stitch_preview::player().unwrap().playing);
        assert_eq!(stitch_preview::player().unwrap().position, None);
        assert!(stitch_preview::command(&mut app, &json!({"pauseAt": true})).is_err());
        assert!(stitch_preview::command(&mut app, &json!({"pauseAt": {"trims": 1}})).is_err());
        assert!(stitch_preview::command(&mut app, &json!({"follow": "yes"})).is_err());
    }

    #[test]
    fn the_view_follows_the_needle_out_of_sight_only_while_playing_and_asked() {
        let mut app = app_with_art();
        let (ctx, mut clock) = (egui::Context::default(), 0.0);
        run(&mut app, "view.stitches", json!({}));
        frames(&mut app, &ctx, &mut clock, 0.05, |_| stitch_preview::player().is_some() && crate::canvas::stitch_view_rect().is_some());
        // Far in on a corner away from the art: the needle is out of sight.
        let away = Point::new(550.0, 380.0);
        let look_away = |app: &mut VectorcraftApp| {
            let v = app.view_mut().unwrap();
            (v.zoom, v.center) = (20.0, away);
        };
        look_away(&mut app);
        stitch_preview::seek(Some(5));
        frames(&mut app, &ctx, &mut clock, 0.05, |_| true);
        assert_eq!(app.view().unwrap().center, away, "a still position moves nothing");
        stitch_preview::command(&mut app, &json!({"speed": 25, "position": 5, "playing": true})).unwrap();
        frames(&mut app, &ctx, &mut clock, 0.01, |a| a.view().unwrap().center != away);
        let c = app.view().unwrap().center;
        assert!(c.x < 110.0 && c.y < 60.0, "the view went to the needle on the rectangle: {c:?}");
        // Off: the view stays where it is put.
        stitch_preview::command(&mut app, &json!({"follow": false, "position": 5, "playing": true})).unwrap();
        look_away(&mut app);
        frames(&mut app, &ctx, &mut clock, 0.01, |_| true);
        frames(&mut app, &ctx, &mut clock, 0.01, |_| true);
        assert_eq!(app.view().unwrap().center, away);
    }
}
