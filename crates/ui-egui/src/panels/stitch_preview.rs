//! Window › Embroidery › Stitch Preview (also Embroidery › Stitch Preview): the stitches the
//! document would sew, drawn as thread (or as a density map), with the plan's figures and a row per
//! colour block, and a stitch-by-stitch playback.
//!
//! The plan and the pictures come from [`vectorcraft_engine::stitch_preview`], built on a worker
//! thread (on the web, where there are none, at once) so the frame never waits for them. The plan
//! is rebuilt about a quarter of a second after the last change of the document or of the Stitch
//! File Options; the picture is redrawn when the view, the toggles or the playback position change.
//! The old picture stays up, marked "updating…", until the new one arrives.
//!
//! The view settings live in `UiState::stitch_preview` (`ui.inspect`); `ui.stitchPreview` sets them,
//! the playback position and play / pause, and shows the panel.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{Color32, Rect, Sense, TextureHandle, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::stitch_preview::{BlockInfo, BuiltPlan, Frame, FrameOptions, Mark, PlanSummary, build_plan};

use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The panel id (Window › Embroidery › Stitch Preview).
pub const ID: &str = "stitchPreview";

/// Seconds the document and options must stay unchanged before the plan is rebuilt.
const DEBOUNCE: f64 = 0.25;
/// Playback speeds, stitches per second.
const SPEEDS: [u32; 5] = [25, 100, 400, 1600, 6400];
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 40.0;

/// What the picture shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    /// Each stitch a lit thread.
    #[default]
    Realistic,
    /// Cells coloured by thread coverage.
    Density,
}

impl View {
    fn parse(s: &str) -> Option<View> {
        match s {
            "realistic" => Some(View::Realistic),
            "density" => Some(View::Density),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            View::Realistic => "realistic",
            View::Density => "density",
        }
    }
}

/// The panel's view settings (kept with the UI state).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub view: View,
    /// A dot at each needle point.
    pub needles: bool,
    /// Jumps as dashed lines, and rings at trims, colour changes and stops.
    pub jumps: bool,
    /// Stitches per second in playback.
    pub speed: u32,
    /// In the canvas's stitch view (View › Stitches), the art shows faintly under the thread.
    pub underlay: bool,
    /// Where a playback stops by itself (the player bar's Pause At).
    pub pause_at: PauseAt,
    /// In the stitch view, the canvas moves to keep the needle in sight during a playback.
    pub follow: bool,
}

/// The machine functions a playback stops at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PauseAt {
    pub color_changes: bool,
    pub trims: bool,
    pub stops: bool,
}

impl PauseAt {
    /// The first place to stop after `from` stitches and at or before `to`, short of the end.
    fn within(self, s: &PlanSummary, from: usize, to: usize) -> Option<usize> {
        let colours = s.blocks.iter().skip(1).filter(|_| self.color_changes).map(|b| b.first);
        let functions = s.marks.iter().filter(|m| if m.1 == Mark::Trim { self.trims } else { self.stops }).map(|m| m.0);
        colours.chain(functions).filter(|at| *at > from && *at <= to && *at < s.stitches).min()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings { view: View::Realistic, needles: false, jumps: false, speed: 100, underlay: false, pause_at: PauseAt::default(), follow: true }
    }
}

/// What the plan is built from: the document (by identity, so a selection change is no change),
/// and the plan options (the Stitch File Options used last).
#[derive(Clone)]
struct PlanKey {
    uid: u64,
    doc: Arc<Document>,
    options: String,
}

impl PartialEq for PlanKey {
    fn eq(&self, o: &PlanKey) -> bool {
        self.uid == o.uid && Arc::ptr_eq(&self.doc, &o.doc) && self.options == o.options
    }
}

struct PlanState {
    key: PlanKey,
    /// The plan, or why there is none.
    plan: Result<Arc<BuiltPlan>, String>,
    /// Counts the plans built, so a frame knows which plan it shows.
    generation: u64,
}

#[derive(Clone, Copy, PartialEq)]
struct FrameKey {
    generation: u64,
    options: FrameOptions,
}

struct FrameState {
    key: FrameKey,
    texture: TextureHandle,
    size: [u32; 2],
    dense_cells: usize,
    max_coverage: f64,
}

type PlanJob = (PlanKey, Receiver<Result<BuiltPlan, String>>);
type FrameJob = (FrameKey, Receiver<Result<Frame, String>>);

#[derive(Default)]
struct Runtime {
    /// The document the state below is for.
    uid: Option<u64>,
    /// The plan key last asked for, and when it changed (seconds of the UI clock).
    wanted: Option<PlanKey>,
    changed_at: f64,
    plan: Option<PlanState>,
    plan_job: Option<PlanJob>,
    generation: u64,
    frame: Option<FrameState>,
    frame_error: Option<(FrameKey, String)>,
    frame_job: Option<FrameJob>,
    /// The stitches shown (`None`: all). While playing, the fractional position is `played`.
    position: Option<usize>,
    playing: bool,
    played: f64,
    last_tick: f64,
    zoom: f32,
    pan: egui::Vec2,
}

thread_local! {
    static RT: RefCell<Runtime> = RefCell::new(Runtime { zoom: 1.0, ..Runtime::default() });
}

fn with_rt<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RT.with(|rt| f(&mut rt.borrow_mut()))
}

/// Runs `work` on a worker thread (at once where there are none) → the receiver of its result. The
/// UI is asked to repaint when it is done.
pub(crate) fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Receiver<Result<T, String>> {
    let (tx, rx) = channel();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = tx.send(work());
        ctx.request_repaint();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let ctx = ctx.clone();
        let sent = tx.clone();
        let started = std::thread::Builder::new().name("stitch-preview".into()).spawn(move || {
            // The receiver is gone when the document was closed: nothing to tell.
            let _ = sent.send(work());
            ctx.request_repaint();
        });
        if let Err(e) = started {
            let _ = tx.send(Err(format!("could not start the preview: {e}")));
        }
    }
    rx
}

/// The plan options: the Stitch File Options last used, as an export takes them.
fn plan_options(app: &VectorcraftApp) -> Value {
    if app.ui.stitch_options.is_object() { crate::dialogs::stitch_options::export_params_of(app.ui.stitch_options.clone()) } else { json!({}) }
}

/// The frame options for the settings and the stitches shown.
pub(crate) fn frame_options(set: &Settings, position: Option<usize>) -> FrameOptions {
    FrameOptions {
        density: set.view == View::Density,
        show_jumps: set.jumps,
        show_needles: set.needles,
        show_functions: set.jumps,
        stitches: position,
    }
}

impl Runtime {
    fn reset(&mut self) {
        let zoom = (self.zoom, self.pan);
        *self = Runtime { zoom: zoom.0, pan: zoom.1, ..Runtime::default() };
    }

    fn total(&self) -> usize {
        match self.plan.as_ref().map(|p| &p.plan) {
            Some(Ok(plan)) => plan.summary.stitches,
            _ => 0,
        }
    }

    /// The stitches shown, within the plan (`None`: all of them).
    fn shown(&self) -> Option<usize> {
        let total = self.total();
        self.position.filter(|p| *p < total)
    }

    /// Whether what is on screen is behind the document, the options or the settings.
    fn updating(&self, key: Option<&PlanKey>, want: Option<FrameKey>) -> bool {
        let plan_current = matches!((self.plan.as_ref(), key), (Some(p), Some(k)) if &p.key == k);
        let frame_current = want.is_none() || self.frame.as_ref().map(|f| f.key) == want || self.frame_error.as_ref().map(|e| e.0) == want;
        self.plan_job.is_some() || self.frame_job.is_some() || !plan_current || !frame_current
    }
}

/// What the canvas's stitch view draws from: the plan on show (with the count of plans built, so
/// a picture knows which plan it is of) or why there is none, and the stitches shown (`None`: all).
pub(crate) struct OnCanvas {
    pub plan: Option<Result<(u64, Arc<BuiltPlan>), String>>,
    pub position: Option<usize>,
}

/// Advance the plan and the playback for the canvas's stitch view (the panel need not be open; its
/// own picture is not drawn for this), and say what there is to draw.
pub(crate) fn for_canvas(app: &VectorcraftApp, ctx: &egui::Context) -> OnCanvas {
    update(app, ctx, &app.ui.stitch_preview, false);
    with_rt(|rt| OnCanvas { plan: rt.plan.as_ref().map(|p| p.plan.clone().map(|plan| (p.generation, plan))), position: rt.shown() })
}

/// The playback speeds as a menu; a choice sets `set.speed`.
pub(crate) fn speed_menu(ui: &mut Ui, set: &mut Settings, id: &str) {
    let speeds: Vec<String> = SPEEDS.iter().map(|s| format!("{s}/s")).collect();
    let labels: Vec<&str> = speeds.iter().map(String::as_str).collect();
    let cur = SPEEDS.iter().position(|s| *s == set.speed).unwrap_or(1);
    if let Some(i) = widgets::dropdown(ui, id, labels.get(cur).copied().unwrap_or(""), &labels, 70.0)
        && let Some(s) = SPEEDS.get(i)
    {
        set.speed = *s;
    }
}

/// What a player shows and moves: the plan's stitches, colour blocks and functions, and where the
/// playback stands.
pub(crate) struct Player {
    pub total: usize,
    /// The stitches shown (`None`: all of them).
    pub position: Option<usize>,
    pub playing: bool,
    pub blocks: Vec<BlockInfo>,
    pub marks: Vec<(usize, Mark)>,
    /// Where the needle stands, in millimetres of the document, while fewer than all stitches show.
    pub needle_mm: Option<(f64, f64)>,
}

/// The playback of the plan on show; `None` while there is no plan.
pub(crate) fn player() -> Option<Player> {
    with_rt(|rt| {
        let plan = rt.plan.as_ref()?.plan.as_ref().ok()?;
        let position = rt.shown();
        Some(Player {
            total: plan.summary.stitches,
            position,
            playing: rt.playing,
            blocks: plan.summary.blocks.clone(),
            marks: plan.summary.marks.clone(),
            needle_mm: position.and_then(|n| plan.needle_mm(n)),
        })
    })
}

/// Show the first `to` stitches (`None`, or the total and more: all of them) and stop playing.
pub(crate) fn seek(to: Option<usize>) {
    with_rt(|rt| {
        let total = rt.total();
        rt.playing = false;
        rt.position = to.filter(|n| *n < total);
        rt.played = to.map_or(total, |n| n.min(total)) as f64;
    });
}

/// Play from where the playback stands (from the start when it is at the end), or pause.
pub(crate) fn toggle_play() {
    with_rt(|rt| {
        if rt.playing {
            rt.playing = false;
        } else if rt.total() > 0 {
            let from = rt.shown().unwrap_or(0);
            (rt.playing, rt.played) = (true, from as f64);
            rt.position = Some(from);
        }
    });
}

/// Move the playback by `by` stitches, within the plan, and stop playing.
pub(crate) fn nudge(by: i64) {
    let (at, total) = with_rt(|rt| (rt.shown().unwrap_or(rt.total()), rt.total()));
    let to = i64::try_from(at).unwrap_or(i64::MAX).saturating_add(by).clamp(0, i64::try_from(total).unwrap_or(i64::MAX));
    seek(usize::try_from(to).ok());
}

/// Move the playback to where the next colour block starts (`forward`; past the last: the end),
/// or back to where this one starts, or the one before when it stands there.
pub(crate) fn step_block(forward: bool) {
    let Some(p) = player() else { return };
    let at = p.position.unwrap_or(p.total);
    let starts = p.blocks.iter().filter(|b| b.stitches > 0).map(|b| b.first);
    let to = if forward { starts.filter(|s| *s > at).min() } else { Some(starts.filter(|s| *s < at).max().unwrap_or(0)) };
    seek(to);
}

/// Advance the state to now: collect finished work, start what is wanted, play. `panel`: the
/// panel is drawing, so its picture of the whole design is wanted too.
fn update(app: &VectorcraftApp, ctx: &egui::Context, set: &Settings, panel: bool) {
    let Some(st) = app.session.active() else { return };
    let (uid, doc) = (st.uid, st.doc.clone());
    let key = PlanKey { uid, doc, options: plan_options(app).to_string() };
    let now = ctx.input(|i| i.time);
    with_rt(|rt| {
        if rt.uid != Some(uid) {
            rt.reset();
            rt.uid = Some(uid);
        }
        // A finished plan.
        if let Some((k, rx)) = rt.plan_job.take() {
            match rx.try_recv() {
                Ok(done) => {
                    rt.generation += 1;
                    rt.plan = Some(PlanState { key: k, plan: done.map(Arc::new), generation: rt.generation });
                }
                Err(TryRecvError::Empty) => rt.plan_job = Some((k, rx)),
                Err(TryRecvError::Disconnected) => {
                    rt.generation += 1;
                    rt.plan = Some(PlanState { key: k, plan: Err("the stitch preview stopped unexpectedly".into()), generation: rt.generation });
                }
            }
        }
        if rt.wanted.as_ref() != Some(&key) {
            rt.wanted = Some(key.clone());
            rt.changed_at = now;
        }
        // Rebuild after a quiet moment; the first plan at once.
        let current = rt.plan.as_ref().is_some_and(|p| p.key == key);
        if !current && rt.plan_job.is_none() {
            let wait = DEBOUNCE - (now - rt.changed_at);
            if rt.plan.is_none() || wait <= 0.0 {
                let (doc, options) = (key.doc.clone(), plan_options(app));
                let rx = spawn(ctx, move || build_plan(&doc, &options));
                rt.plan_job = Some((key.clone(), rx));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.01)));
            }
        }
        // Playback.
        let total = rt.total();
        if rt.playing && total > 0 {
            let dt = (now - rt.last_tick).clamp(0.0, 0.25);
            // Non-negative and at most the total, so the casts are exact.
            let from = rt.played.max(0.0).floor() as usize;
            rt.played += dt * f64::from(set.speed);
            let to = (rt.played.max(0.0).floor() as usize).min(total);
            let pause = match rt.plan.as_ref().map(|p| &p.plan) {
                Some(Ok(plan)) => set.pause_at.within(&plan.summary, from, to),
                _ => None,
            };
            if let Some(at) = pause {
                (rt.playing, rt.position, rt.played) = (false, Some(at), at as f64);
                ctx.request_repaint();
            } else if rt.played >= total as f64 {
                (rt.playing, rt.position) = (false, None);
            } else {
                // Within 0..total, so the cast is exact.
                rt.position = Some(rt.played.floor() as usize);
                ctx.request_repaint();
            }
        } else {
            rt.playing = false;
        }
        rt.last_tick = now;
        if !panel {
            return;
        }
        // A finished frame.
        if let Some((k, rx)) = rt.frame_job.take() {
            match rx.try_recv() {
                Ok(Ok(f)) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                    let texture = ctx.load_texture("stitch-preview", image, egui::TextureOptions::LINEAR);
                    rt.frame =
                        Some(FrameState { key: k, texture, size: [f.width, f.height], dense_cells: f.dense_cells, max_coverage: f.max_coverage });
                    rt.frame_error = None;
                }
                Ok(Err(e)) => rt.frame_error = Some((k, e)),
                Err(TryRecvError::Empty) => rt.frame_job = Some((k, rx)),
                Err(TryRecvError::Disconnected) => rt.frame_error = Some((k, "the stitch preview stopped unexpectedly".into())),
            }
        }
        // The picture the plan on show wants.
        let shown = rt.plan.as_ref().and_then(|p| p.plan.as_ref().ok().map(|plan| (p.generation, plan.clone())));
        if let Some((generation, plan)) = shown {
            let want = FrameKey { generation, options: frame_options(set, rt.shown()) };
            let have = rt.frame.as_ref().map(|f| f.key) == Some(want) || rt.frame_error.as_ref().map(|e| e.0) == Some(want);
            if !have && rt.frame_job.is_none() {
                let o = want.options;
                rt.frame_job = Some((want, spawn(ctx, move || plan.frame(&o))));
            }
        }
    });
}

/// A length of thread, in mm below a metre.
fn length_text(mm: f64) -> String {
    if mm >= 1000.0 { format!("{:.2} m", mm / 1000.0) } else { format!("{mm:.0} mm") }
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The picture, fitted, zoomed with the wheel and panned by dragging; double-click fits it.
fn picture(ui: &mut Ui, t: &Tokens, height: f32, updating: bool, note: Option<&str>) {
    let (area, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(60.0), height), Sense::click_and_drag());
    let painter = ui.painter().with_clip_rect(area);
    painter.rect_filled(area, 3.0, t.panel_darker);
    with_rt(|rt| {
        if resp.double_clicked() {
            (rt.zoom, rt.pan) = (1.0, egui::Vec2::ZERO);
        }
        if resp.dragged() {
            rt.pan += resp.drag_delta();
        }
        if let Some(f) = &rt.frame {
            let size = vec2(f.size[0] as f32, f.size[1] as f32);
            let fit = (area.width() / size.x).min(area.height() / size.y).max(1e-3);
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if let (true, Some(p)) = (scroll != 0.0, ui.input(|i| i.pointer.hover_pos())) {
                    let zoom = (rt.zoom * (scroll * 0.0025).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
                    let f = zoom / rt.zoom;
                    // The point under the pointer stays under it.
                    rt.pan = p - area.center() - (p - area.center() - rt.pan) * f;
                    rt.zoom = zoom;
                }
            }
            let shown = size * fit * rt.zoom;
            let image = Rect::from_center_size(area.center() + rt.pan, shown);
            painter.image(f.texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if let Some(n) = note {
            painter.text(area.center(), egui::Align2::CENTER_CENTER, n, egui::FontId::proportional(12.5), t.text_dim);
        }
    });
    if updating {
        painter.text(area.left_top() + vec2(8.0, 6.0), egui::Align2::LEFT_TOP, tl!("updating…"), egui::FontId::proportional(12.0), t.text_dim);
    }
}

fn summary_rows(ui: &mut Ui, s: &PlanSummary, shown: Option<usize>, dense: Option<(usize, f64)>) {
    egui::Grid::new("stitch-preview-summary").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        let row = |ui: &mut Ui, name: &str, value: String| {
            widgets::dim_label(ui, name);
            ui.label(value);
            ui.end_row();
        };
        let stitches = match shown {
            Some(n) => format!("{n} / {}", s.stitches),
            None => s.stitches.to_string(),
        };
        row(ui, tl!("Stitches:"), stitches);
        row(ui, tl!("Color changes:"), s.color_changes.to_string());
        row(ui, tl!("Trims:"), s.trims.to_string());
        row(ui, tl!("Jumps:"), s.jumps.to_string());
        row(ui, tl!("Size:"), format!("{:.1} × {:.1} mm", s.width_mm, s.height_mm));
        if let Some((cells, max)) = dense {
            row(ui, tl!("Peak density:"), format!("{max:.2} ({cells} too dense)"));
        }
    });
    for w in &s.warnings {
        ui.label(egui::RichText::new(format!("⚠ {w}")).size(11.5).color(Color32::from_rgb(214, 140, 30)));
    }
}

/// One row per thread block; the one holding the stitch shown is highlighted, and a click moves the
/// playback to the end of that block.
fn block_list(ui: &mut Ui, t: &Tokens, blocks: &[BlockInfo], active: Option<usize>) -> Option<usize> {
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("stitch-preview-blocks").max_height(150.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, b) in blocks.iter().enumerate() {
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
            if active == Some(i) {
                ui.painter().rect_filled(row, 3.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(row, 3.0, t.hover);
            }
            let swatch = Rect::from_min_size(row.left_center() + vec2(4.0, -7.0), vec2(14.0, 14.0));
            ui.painter().rect_filled(swatch, 2.0, rgb(b.rgb));
            ui.painter().rect_stroke(swatch, 2.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let font = egui::FontId::proportional(12.0);
            ui.painter().text(row.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, format!("{}", i + 1), font.clone(), t.text);
            let detail = format!("{} · {}", crate::i18n::tn(b.stitches as u64, "{n} stitch", "{n} stitches"), length_text(b.length_mm));
            ui.painter().text(row.right_center() - vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, detail, font, t.text_dim);
            if resp.clicked() {
                clicked = Some(b.first + b.stitches);
            }
        }
    });
    clicked
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        super::empty_state(ui, "pen-tool", tl!("No document"), tl!("Open a document to preview its stitches."));
        return;
    }
    let mut set = app.ui.stitch_preview.clone();
    update(app, ui.ctx(), &set, true);
    let mut position = with_rt(|rt| rt.position);
    let (total, playing) = with_rt(|rt| (rt.total(), rt.playing));
    let mut play_clicked = false;
    let mut canvas_view = None;

    ui.horizontal_wrapped(|ui| {
        let labels = [tl!("Thread"), tl!("Density")];
        let current = if set.view == View::Density { 1 } else { 0 };
        if let Some(i) = widgets::dropdown(ui, "stitch-preview-view", labels[current], &labels, 96.0) {
            set.view = if i == 1 { View::Density } else { View::Realistic };
        }
        ui.checkbox(&mut set.needles, tl!("Needle points")).on_hover_text(tl!("A dot at each needle point"));
        ui.checkbox(&mut set.jumps, tl!("Jumps and trims")).on_hover_text(tl!("Jumps as dashed lines; rings at trims, colour changes and stops"));
        let mut on_canvas = app.stitch_view();
        if ui
            .checkbox(&mut on_canvas, tl!("On the canvas"))
            .on_hover_text(tl!("Show the stitches on the canvas instead of the art (View › Stitches)"))
            .changed()
        {
            canvas_view = Some(on_canvas);
        }
        ui.add_enabled(on_canvas, egui::Checkbox::new(&mut set.underlay, tl!("Artwork underlay")))
            .on_hover_text(tl!("On the canvas, show the art faintly under the thread"));
    });
    if let Some(on) = canvas_view {
        // The command only fails without a document, and the panel shows nothing then.
        app.run("view.stitches", json!({ "on": on })).ok();
    }
    ui.add_space(4.0);

    let height = {
        let avail = ui.available_height();
        if avail.is_finite() { (avail - 270.0).clamp(180.0, 560.0) } else { 280.0 }
    };
    let (key, want, note, summary, dense, error) = with_rt(|rt| {
        let want = rt
            .plan
            .as_ref()
            .and_then(|p| p.plan.as_ref().ok().map(|_| FrameKey { generation: p.generation, options: frame_options(&set, rt.shown()) }));
        let summary = rt.plan.as_ref().and_then(|p| p.plan.as_ref().ok()).map(|p| p.summary.clone());
        let error = match rt.plan.as_ref().map(|p| &p.plan) {
            Some(Err(e)) => Some(e.clone()),
            _ => rt.frame_error.as_ref().map(|e| e.1.clone()),
        };
        let dense = match (&rt.frame, set.view) {
            (Some(f), View::Density) => Some((f.dense_cells, f.max_coverage)),
            _ => None,
        };
        (rt.wanted.clone(), want, rt.plan.is_none().then(|| tl!("Building the stitches…").to_string()), summary, dense, error)
    });
    let updating = with_rt(|rt| rt.updating(key.as_ref(), want));
    let note = error.clone().or(note);
    picture(ui, &t, height, updating && error.is_none(), note.as_deref());

    // Playback: here while the canvas shows the art; in the stitch view the player bar under the
    // canvas has it.
    if !app.stitch_view() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (label, tip) =
                if playing { (tl!("Pause"), tl!("Pause the playback")) } else { (tl!("Play"), tl!("Play the stitches in sewing order")) };
            if ui.add_enabled_ui(total > 0, |ui| widgets::flat_button(ui, label, 52.0).on_hover_text(tip)).inner.clicked() {
                play_clicked = true;
            }
            let mut at = position.map_or(total, |p| p.min(total));
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(60.0);
            let slider = ui.add_enabled(total > 0, egui::Slider::new(&mut at, 0..=total).show_value(false).trailing_fill(true));
            if slider.changed() {
                position = if at >= total { None } else { Some(at) };
                with_rt(|rt| (rt.position, rt.played) = (position, at as f64));
            }
            speed_menu(ui, &mut set, "stitch-preview-speed");
        });
        if play_clicked {
            toggle_play();
        }
    }
    ui.add_space(4.0);

    if let Some(s) = summary {
        let shown = with_rt(|rt| rt.shown());
        summary_rows(ui, &s, shown, dense);
        widgets::divider(ui);
        // The block holding the stitch shown (the last one when all of them are).
        let n = shown.unwrap_or(s.stitches);
        let active = s.block_at(n);
        if let Some(end) = block_list(ui, &t, &s.blocks, active) {
            with_rt(|rt| {
                rt.playing = false;
                rt.position = if end >= s.stitches { None } else { Some(end) };
                rt.played = end as f64;
            });
        }
    }
    if set != app.ui.stitch_preview {
        app.ui.stitch_preview = set;
    }
}

/// `ui.stitchPreview {view?, needles?, jumps?, speed?, position?, playing?}`: set the panel's view,
/// toggles, playback position (a stitch count, or null / "end" for all of them) and play / pause,
/// and show it → its state.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut set = app.ui.stitch_preview.clone();
    if let Some(v) = p.get("view") {
        let v = v.as_str().and_then(View::parse).ok_or("view must be realistic or density")?;
        set.view = v;
    }
    for (key, field) in [("needles", &mut set.needles), ("jumps", &mut set.jumps), ("underlay", &mut set.underlay)] {
        if let Some(v) = p.get(key) {
            *field = v.as_bool().ok_or_else(|| format!("{key} must be true or false"))?;
        }
    }
    if let Some(v) = p.get("speed") {
        set.speed = v
            .as_u64()
            .filter(|s| (1..=100_000).contains(s))
            .and_then(|s| u32::try_from(s).ok())
            .ok_or("speed must be a whole number of stitches per second, 1–100000")?;
    }
    if let Some(v) = p.get("follow") {
        set.follow = v.as_bool().ok_or("follow must be true or false")?;
    }
    if let Some(v) = p.get("pauseAt") {
        let o = v.as_object().ok_or("pauseAt must be {colorChanges?, trims?, stops?}")?;
        for (key, field) in
            [("colorChanges", &mut set.pause_at.color_changes), ("trims", &mut set.pause_at.trims), ("stops", &mut set.pause_at.stops)]
        {
            if let Some(v) = o.get(key) {
                *field = v.as_bool().ok_or_else(|| format!("pauseAt.{key} must be true or false"))?;
            }
        }
    }
    let position = match p.get("position") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if s == "end" => Some(None),
        Some(v) => Some(Some(v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("position must be a stitch count, or null for all of them")?)),
    };
    let playing = match p.get("playing") {
        None => None,
        Some(v) => Some(v.as_bool().ok_or("playing must be true or false")?),
    };
    app.ui.stitch_preview = set;
    with_rt(|rt| {
        if let Some(pos) = position {
            rt.position = pos;
            rt.played = pos.unwrap_or(0) as f64;
        }
        if let Some(on) = playing {
            if on && position.is_none() {
                let from = rt.shown().unwrap_or(0);
                rt.played = from as f64;
                rt.position = Some(from);
            }
            rt.playing = on;
        }
    });
    app.ui.open_panel = Some(ID.into());
    app.ui.dock = true;
    Ok(state(app))
}

/// Why there is no plan to show, when that is so.
#[cfg(test)]
pub(crate) fn state_message() -> Option<String> {
    with_rt(|rt| match rt.plan.as_ref().map(|p| &p.plan) {
        Some(Err(e)) => Some(e.clone()),
        _ => None,
    })
}

/// What the panel is showing (`ui.stitchPreview`'s answer): `status` is `ready`, `updating`, `error`
/// or `none` (no document).
pub(crate) fn state(app: &VectorcraftApp) -> Value {
    let set = &app.ui.stitch_preview;
    let key = with_rt(|rt| rt.wanted.clone());
    with_rt(|rt| {
        let (status, message) = match rt.plan.as_ref().map(|p| &p.plan) {
            _ if app.session.active().is_none() => ("none", None),
            Some(Err(e)) if !rt.plan_job.is_some() => ("error", Some(e.clone())),
            None => ("updating", None),
            _ => match rt.frame_error.as_ref() {
                Some((_, e)) => ("error", Some(e.clone())),
                None if rt.updating(key.as_ref(), None) => ("updating", None),
                None => ("ready", None),
            },
        };
        let mut v = json!({
            "panel": ID,
            "status": status,
            "view": set.view.id(),
            "needles": set.needles,
            "jumps": set.jumps,
            "speed": set.speed,
            "underlay": set.underlay,
            "onCanvas": app.stitch_view(),
            "follow": set.follow,
            "pauseAt": {"colorChanges": set.pause_at.color_changes, "trims": set.pause_at.trims, "stops": set.pause_at.stops},
            "position": rt.shown(),
            "playing": rt.playing,
            "stitches": rt.total(),
        });
        if let Some(m) = message {
            v["message"] = json!(m);
        }
        if let Some(Ok(plan)) = rt.plan.as_ref().map(|p| &p.plan) {
            let s = &plan.summary;
            v["colorChanges"] = json!(s.color_changes);
            v["trims"] = json!(s.trims);
            v["jumpCount"] = json!(s.jumps);
            v["widthMm"] = json!(s.width_mm);
            v["heightMm"] = json!(s.height_mm);
            v["blocks"] = json!(s.blocks.len());
            v["warnings"] = json!(s.warnings);
        }
        if let Some(f) = &rt.frame {
            v["image"] = json!({"width": f.size[0], "height": f.size[1]});
        }
        if app.stitch_view() {
            v["canvas"] = crate::canvas::stitch_view_state();
        }
        v
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
        app.session.execute(id, &p).unwrap()
    }

    /// Run the panel for `n` headless frames, waiting for the worker between them: the texts drawn
    /// in the last one.
    fn frames(app: &mut VectorcraftApp, until: impl Fn(&Value) -> bool) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        let mut last = vec![];
        for i in 0..400 {
            let input = egui::RawInput { time: Some(f64::from(i) * 0.05), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(app, ui));
            out.textures_delta.clear();
            last = vec![];
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut last));
            if until(&state(app)) {
                return last;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the panel did not settle: {} {last:?}", state(app));
    }

    fn app_with_art() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 600, "height": 400}));
        run(&mut app, "paint.setFill", json!({"none": true}));
        run(&mut app, "paint.setStroke", json!({"color": "#ff0000"}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50}));
        run(&mut app, "select.none", json!({}));
        run(&mut app, "paint.setStroke", json!({"color": "#0000ff"}));
        run(&mut app, "shape.ellipse", json!({"x": 20, "y": 80, "width": 60, "height": 40}));
        app
    }

    #[test]
    fn the_panel_builds_the_plan_off_the_frame_and_shows_its_figures_and_blocks() {
        let mut app = app_with_art();
        let first = ui_command(&mut app, json!({}));
        assert_eq!(first["status"], "updating", "the plan is built on a worker, not in the command: {first}");
        let texts = frames(&mut app, |s| s["status"] == "ready" && s["image"].is_object());
        let s = state(&app);
        assert!(s["stitches"].as_u64().unwrap() > 20, "{s}");
        assert_eq!(s["blocks"], 2);
        for label in ["Stitches:", "Trims:", "Size:"] {
            assert!(texts.iter().any(|t| t == label), "{label} in {texts:?}");
        }
        assert!(texts.iter().any(|t| t.contains("stitches")), "a row per block: {texts:?}");
        assert!(!texts.iter().any(|t| t == "updating…"));
        assert!(app.ui.open_panel.as_deref() == Some(ID));
    }

    fn ui_command(app: &mut VectorcraftApp, p: Value) -> Value {
        command(app, &p).unwrap()
    }

    #[test]
    fn view_toggles_and_playback_position_redraw_without_rebuilding_the_plan() {
        let mut app = app_with_art();
        ui_command(&mut app, json!({}));
        frames(&mut app, |s| s["status"] == "ready" && s["image"].is_object());
        let total = state(&app)["stitches"].as_u64().unwrap();
        let generation = with_rt(|rt| rt.generation);
        let s = ui_command(&mut app, json!({"view": "density", "needles": true, "jumps": true, "position": 10}));
        assert_eq!((s["view"].as_str(), s["needles"].as_bool(), s["position"].as_u64()), (Some("density"), Some(true), Some(10)));
        frames(&mut app, |s| s["status"] == "ready");
        assert_eq!(with_rt(|rt| rt.generation), generation, "the plan was not rebuilt");
        let key = with_rt(|rt| rt.frame.as_ref().map(|f| f.key));
        assert_eq!(key.map(|k| (k.options.density, k.options.stitches)), Some((true, Some(10))));
        // Past the end is all of the stitches; null too.
        let s = ui_command(&mut app, json!({"position": total + 50}));
        assert!(s["position"].is_null(), "{s}");
        assert!(ui_command(&mut app, json!({"position": null}))["position"].is_null());
        assert!(command(&mut app, &json!({"view": "wireframe"})).is_err());
        assert!(command(&mut app, &json!({"position": -1})).is_err());
        assert!(command(&mut app, &json!({"playing": "yes"})).is_err());
    }

    #[test]
    fn playing_advances_the_position_and_stops_at_the_end() {
        let mut app = app_with_art();
        ui_command(&mut app, json!({"speed": 400}));
        frames(&mut app, |s| s["status"] == "ready" && s["image"].is_object());
        let s = ui_command(&mut app, json!({"playing": true, "position": 0}));
        assert_eq!(s["playing"], true);
        // 400 stitches a second over a few seconds of frame time reaches the end.
        frames(&mut app, |s| s["playing"] == false);
        assert!(state(&app)["position"].is_null(), "back at the end: {}", state(&app));
    }

    #[test]
    fn a_changed_document_is_rebuilt_after_the_quiet_moment_and_an_empty_one_says_why() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        run(&mut app, "file.new", json!({"width": 600, "height": 400}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 20}));
        ui_command(&mut app, json!({}));
        // A filled rectangle with the default (stroke) paint of a new document has stitches or says why.
        let first = frames(&mut app, |s| s["status"] == "ready" || s["status"] == "error");
        let generation = with_rt(|rt| rt.generation);
        assert!(generation >= 1, "{first:?}");
        run(&mut app, "paint.setFill", json!({"none": true}));
        run(&mut app, "paint.setStroke", json!({"color": "#009900"}));
        run(&mut app, "shape.ellipse", json!({"x": 0, "y": 0, "width": 60, "height": 60}));
        frames(&mut app, |s| s["status"] == "ready" && s["image"].is_object());
        assert!(with_rt(|rt| rt.generation) > generation, "the new art was planned");
        // Selecting something else is no change of the document.
        let generation = with_rt(|rt| rt.generation);
        run(&mut app, "select.all", json!({}));
        run(&mut app, "select.none", json!({}));
        frames(&mut app, |s| s["status"] == "ready");
        assert_eq!(with_rt(|rt| rt.generation), generation);
    }
}
