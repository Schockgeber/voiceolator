//! The Voiceolator window: full window (Live + Files tabs), slim bar and tiny pill.

use crate::config::{Config, EngineChoice, ViewMode};
use crate::files::{self, Files};
use crate::platform::{self, OutputInfo, SourceInfo};
use crate::theme::*;
use crate::tray::{Tray, TrayState};
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use vo_audio::{AudioHost, HostConfig, Level};
use vo_dsp::Preset;

pub const FULL_SIZE: [f32; 2] = [480.0, 800.0];
pub const FULL_MIN: [f32; 2] = [420.0, 520.0];
pub const SLIM_SIZE: [f32; 2] = [560.0, 46.0];
pub const PILL_SIZE: [f32; 2] = [196.0, 40.0];

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Stopped,
    Running,
    Measuring,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tab {
    Live,
    Files,
}

/// Commands from the tray, global hotkeys and a second app start.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // not every platform sends every command
pub enum UiCommand {
    ToggleIsolation,
    ToggleMute,
    ShowWindow,
    ToggleWindow,
    StartStop,
    SetView(ViewMode),
    /// Files given on the command line / dropped on the program icon.
    AddFiles(Vec<std::path::PathBuf>),
    Quit,
}

struct Message {
    at: Instant,
    level: Level,
    text: String,
}

type LatencyResult = Result<(f64, f64, f64, usize, String), String>;

pub struct VoiceolatorApp {
    cfg: Config,
    sources: Vec<SourceInfo>,
    outputs: Vec<OutputInfo>,
    last_refresh: Instant,
    host: Option<AudioHost>,
    engine_label: String,
    hide: Option<platform::HideGuard>,
    hide_reported: bool,
    messages: VecDeque<Message>,
    status: Status,
    latency_job: Option<Receiver<LatencyResult>>,
    resume_after_measure: bool,
    measured: Option<String>,
    meters: (f32, f32, f32),
    commands: Receiver<UiCommand>,
    config_dirty: Option<Instant>,
    applied_on_top: Option<bool>,
    applied_view: Option<ViewMode>,
    tab: Tab,
    files: Files,
    tray: Option<Tray>,
    hidden: bool,
    /// Just asked the window to come back; ignore a stale "minimized" until then.
    restoring: Option<Instant>,
    quitting: bool,
    close_dialog: bool,
}

impl VoiceolatorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, mut cfg: Config, commands: Receiver<UiCommand>, tray: Option<Tray>) -> Self {
        style(&cc.egui_ctx);
        let mut messages = VecDeque::new();
        // Crash recovery: give back any app sound a previous run turned down or muted.
        let n = platform::recover(&cfg.muted_sessions);
        if n > 0 {
            messages.push_back(Message { at: Instant::now(), level: Level::Info, text: format!("Restored the sound of {n} app session(s) from the last run.") });
        }
        if !cfg.muted_sessions.is_empty() {
            cfg.muted_sessions.clear();
            cfg.save();
        }
        let measured = cfg.last_measured_latency_ms.map(|m| format!("{m:.0} ms (last measured)"));
        let files = Files::new(&cc.egui_ctx, &cfg);
        let view = cfg.view;
        let mut app = Self {
            cfg,
            sources: Vec::new(),
            outputs: Vec::new(),
            last_refresh: Instant::now() - Duration::from_secs(60),
            host: None,
            engine_label: String::new(),
            hide: None,
            hide_reported: false,
            messages,
            status: Status::Stopped,
            latency_job: None,
            resume_after_measure: false,
            measured,
            meters: (-90.0, -90.0, 0.0),
            commands,
            config_dirty: None,
            applied_on_top: None,
            // main() already opened the window in the saved layout
            applied_view: Some(view),
            tab: Tab::Live,
            files,
            tray,
            hidden: false,
            restoring: None,
            quitting: false,
            close_dialog: false,
        };
        app.refresh_lists();
        if platform::demo_mode() {
            app.say(Level::Info, "Demo mode: simulated audio devices.");
        }
        if app.cfg.start_automatically && app.cfg.source_exe.is_some() {
            app.start();
        }
        app
    }

    fn say(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        log::info!("[{level:?}] {text}");
        if self.messages.back().map(|m| m.text == text).unwrap_or(false) {
            self.messages.back_mut().unwrap().at = Instant::now();
            return;
        }
        self.messages.push_back(Message { at: Instant::now(), level, text });
        while self.messages.len() > 6 {
            self.messages.pop_front();
        }
    }

    fn mark_dirty(&mut self) {
        self.config_dirty = Some(Instant::now());
    }

    fn refresh_lists(&mut self) {
        self.last_refresh = Instant::now();
        match platform::list_sources() {
            Ok(mut s) => {
                // keep the configured source visible even if it is not playing right now
                if let Some(exe) = &self.cfg.source_exe {
                    if !s.iter().any(|x| x.exe.eq_ignore_ascii_case(exe)) {
                        s.push(SourceInfo { exe: exe.clone(), pid: 0, playing: false });
                    }
                }
                self.sources = s;
            }
            Err(e) => self.say(Level::Error, format!("Could not list audio apps: {e:#}")),
        }
        match platform::list_outputs() {
            Ok(o) => self.outputs = o,
            Err(e) => self.say(Level::Error, format!("Could not list output devices: {e:#}")),
        }
        if self.cfg.source_exe.is_none() {
            // Pre-select the first browser that is playing audio.
            let browsers = ["firefox", "chrome", "msedge", "opera", "brave", "vivaldi", "twitch"];
            if let Some(s) = self.sources.iter().find(|s| s.playing && browsers.iter().any(|b| s.exe.to_lowercase().starts_with(b))) {
                self.cfg.source_exe = Some(s.exe.clone());
            }
        }
    }

    fn selected_output(&self) -> OutputInfo {
        self.outputs
            .iter()
            .find(|o| o.id == self.cfg.output_id)
            .cloned()
            .unwrap_or_else(|| OutputInfo { id: self.cfg.output_id.clone(), name: self.cfg.output_name.clone().unwrap_or_else(|| "Default output".into()) })
    }

    fn start(&mut self) {
        let Some(exe) = self.cfg.source_exe.clone() else {
            self.say(Level::Warn, "Choose the app that plays the stream (e.g. your browser) first.");
            return;
        };
        if !platform::app_running(&exe) {
            self.say(Level::Warn, format!("{} is not running. Open the stream first, then press Start.", pretty(&exe)));
            return;
        }
        let out = self.selected_output();
        let (engine, warn) = platform::engine_kind(self.cfg.engine);
        if let Some(w) = warn {
            self.say(Level::Warn, w);
        }
        self.engine_label = engine.label().to_string();
        let (input, output, hide) = match platform::factories(&exe, &out, self.cfg.buffer_ms, self.cfg.hide_original) {
            Ok(f) => f,
            Err(e) => {
                self.say(Level::Error, format!("Cannot open audio: {e:#}"));
                return;
            }
        };
        match AudioHost::start(HostConfig { input, output, engine, settings: self.cfg.settings(), output_buffer_ms: self.cfg.buffer_ms, record: None }) {
            Ok(h) => {
                self.host = Some(h);
                self.status = Status::Running;
                self.say(Level::Info, format!("Listening to {} on {}", pretty(&exe), out.name));
                self.hide = hide;
                self.hide_reported = false;
            }
            Err(e) => self.say(Level::Error, format!("Could not start: {e:#}")),
        }
    }

    fn stop(&mut self) {
        if let Some(h) = self.host.take() {
            h.stop();
        }
        self.restore_original();
        self.status = Status::Stopped;
        self.meters = (-90.0, -90.0, 0.0);
    }

    fn start_stop(&mut self) {
        match self.status {
            Status::Running => {
                self.stop();
                self.say(Level::Info, "Stopped. Original audio restored.");
            }
            Status::Stopped => self.start(),
            Status::Measuring => {}
        }
    }

    fn restart_if_running(&mut self) {
        if self.status == Status::Running {
            self.stop();
            self.start();
        }
    }

    fn restore_original(&mut self) {
        if let Some(h) = self.hide.take() {
            h.restore();
        }
    }

    fn push_settings(&mut self) {
        if let Some(h) = &self.host {
            h.set_settings(self.cfg.settings());
        }
        self.mark_dirty();
    }

    fn toggle_isolation(&mut self) {
        self.cfg.isolation_enabled = !self.cfg.isolation_enabled;
        self.push_settings();
    }

    fn toggle_mute(&mut self) {
        self.cfg.muted = !self.cfg.muted;
        self.push_settings();
    }

    fn cycle_preset(&mut self) {
        let next = match self.cfg.preset() {
            Preset::Balanced => Preset::VoicePriority,
            Preset::VoicePriority => Preset::MaximumSuppression,
            _ => Preset::Balanced,
        };
        self.cfg.apply_preset(next);
        self.push_settings();
    }

    fn start_measurement(&mut self) {
        self.resume_after_measure = self.status == Status::Running;
        self.stop();
        let (engine, _) = platform::engine_kind(self.cfg.engine);
        let out = self.selected_output();
        let buffer = self.cfg.buffer_ms;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = platform::measure_latency(engine, &out, buffer)
                .map(|r| (r.median_ms, r.min_ms, r.max_ms, r.samples_ms.len(), r.engine))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
        self.latency_job = Some(rx);
        self.status = Status::Measuring;
        self.say(Level::Info, "Measuring latency: you will hear about 10 short beeps…");
    }

    // ---------- window: tray, hide/show, layouts ----------

    fn tray_ok(&self) -> bool {
        self.tray.as_ref().map(|t| t.available()).unwrap_or(false)
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        crate::tray::show_native();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.hidden = false;
        self.restoring = Some(Instant::now());
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        if self.tray_ok() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    fn set_view(&mut self, ctx: &egui::Context, v: ViewMode) {
        if v == self.cfg.view {
            return;
        }
        // remember where the current layout was
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let Some(r) = outer {
            let p = Some([r.min.x, r.min.y]);
            match self.cfg.view {
                ViewMode::Full => {
                    self.cfg.full_pos = p;
                    self.cfg.full_size = inner.map(|r| [r.width(), r.height()]);
                }
                ViewMode::Slim => self.cfg.slim_pos = p,
                ViewMode::Pill => self.cfg.pill_pos = p,
            }
        }
        self.cfg.view = v;
        self.applied_view = None;
        self.mark_dirty();
    }

    fn apply_view(&mut self, ctx: &egui::Context) {
        if self.applied_view == Some(self.cfg.view) {
            return;
        }
        self.applied_view = Some(self.cfg.view);
        self.applied_on_top = None;
        use egui::ViewportCommand as C;
        let (size, min, pos, deco) = match self.cfg.view {
            ViewMode::Full => (self.cfg.full_size.unwrap_or(FULL_SIZE), FULL_MIN, self.cfg.full_pos, true),
            ViewMode::Slim => (SLIM_SIZE, SLIM_SIZE, self.cfg.slim_pos, false),
            ViewMode::Pill => (PILL_SIZE, PILL_SIZE, self.cfg.pill_pos, false),
        };
        ctx.send_viewport_cmd(C::Decorations(deco));
        ctx.send_viewport_cmd(C::Resizable(deco));
        ctx.send_viewport_cmd(C::MinInnerSize(min.into()));
        ctx.send_viewport_cmd(C::InnerSize(size.into()));
        if let Some(p) = pos {
            ctx.send_viewport_cmd(C::OuterPosition(p.into()));
        }
    }

    fn handle_command(&mut self, ctx: &egui::Context, c: UiCommand) {
        match c {
            UiCommand::ToggleIsolation => {
                self.toggle_isolation();
                let t = if self.cfg.isolation_enabled { "Voice isolation ON" } else { "Voice isolation OFF (original sound)" };
                self.say(Level::Info, t);
            }
            UiCommand::ToggleMute => self.toggle_mute(),
            UiCommand::ShowWindow => self.show_window(ctx),
            UiCommand::ToggleWindow => {
                let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
                if self.hidden || minimized {
                    self.show_window(ctx);
                } else {
                    self.hide_window(ctx);
                }
            }
            UiCommand::StartStop => self.start_stop(),
            UiCommand::SetView(v) => {
                self.show_window(ctx);
                self.set_view(ctx, v);
            }
            UiCommand::AddFiles(v) => {
                self.files.add(v);
                self.tab = Tab::Files;
                self.show_window(ctx);
                self.set_view(ctx, ViewMode::Full);
            }
            UiCommand::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn window_events(&mut self, ctx: &egui::Context) {
        let (close, minimized) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized.unwrap_or(false)));
        if close && !self.quitting && self.cfg.close_to_tray && self.tray_ok() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.cfg.close_asked {
                self.hide_window(ctx);
            } else {
                self.close_dialog = true;
            }
        }
        if let Some(t) = self.restoring {
            if !minimized || t.elapsed() > Duration::from_secs(2) {
                self.restoring = None;
            }
        }
        if minimized && !self.hidden && self.restoring.is_none() && self.cfg.minimize_to_tray && self.tray_ok() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }
        if self.hidden && !self.tray_ok() {
            // the tray went away: never leave the window unreachable
            self.show_window(ctx);
        }
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            self.files.add(dropped);
            self.tab = Tab::Files;
            if self.cfg.view != ViewMode::Full {
                self.set_view(ctx, ViewMode::Full);
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let cmds: Vec<UiCommand> = self.commands.try_iter().collect();
        for c in cmds {
            self.handle_command(ctx, c);
        }
        self.window_events(ctx);
        self.apply_view(ctx);
        if let Some(rx) = &self.latency_job {
            if let Ok(r) = rx.try_recv() {
                self.latency_job = None;
                self.status = Status::Stopped;
                match r {
                    Ok((med, min, max, n, eng)) => {
                        self.measured = Some(format!("{med:.0} ms (range {min:.0}–{max:.0}, {n} beeps, {eng})"));
                        self.cfg.last_measured_latency_ms = Some(med);
                        self.mark_dirty();
                        self.say(Level::Info, format!("Measured end-to-end latency: {med:.0} ms."));
                    }
                    Err(e) => self.say(Level::Error, format!("Latency measurement failed: {e}")),
                }
                if self.resume_after_measure {
                    self.start();
                }
            }
        }
        let events: Vec<_> = self.host.as_ref().map(|h| h.poll_events()).unwrap_or_default();
        for e in events {
            // Routine start-up notices are not interesting to most users.
            if e.level == Level::Info && (e.message == "Output started" || e.message == "Capturing audio") {
                continue;
            }
            if e.message.contains("ONNX Runtime") {
                self.say(
                    Level::Warn,
                    "The Quality engine needs the Microsoft Visual C++ Redistributable (x64): aka.ms/vs/17/release/vc_redist.x64.exe. Using the Light engine for now.",
                );
                continue;
            }
            self.say(e.level, e.message);
        }
        // file processing runs in the background, also while the window is hidden
        let mut dirty = false;
        {
            let (engine, label, settings) = self.file_engine();
            let mut env = files::Env { cfg: &mut self.cfg, engine, engine_label: label, settings, dirty: &mut dirty };
            self.files.poll(ctx, &mut env);
        }
        if dirty {
            self.mark_dirty();
        }
        if self.status == Status::Running {
            if let Some(h) = &self.hide {
                if !self.hide_reported {
                    match h.compensated() {
                        Some(true) => {
                            self.hide_reported = true;
                            self.say(Level::Info, "The original stream sound is hidden. It comes back when you press Stop.");
                        }
                        Some(false) => {
                            self.hide_reported = true;
                            self.say(Level::Info, "The original stream sound is turned down while Voiceolator runs.");
                        }
                        None => {}
                    }
                }
            }
            if let Some(h) = &self.host {
                let t = h.telemetry();
                let decay = |old: f32, new: f32| if new > old { new } else { old - 1.5 };
                self.meters.0 = decay(self.meters.0, t.input_peak_db.get().max(-90.0));
                self.meters.1 = decay(self.meters.1, t.output_peak_db.get().max(-90.0));
                self.meters.2 = t.speech_activity.get();
            }
            ctx.request_repaint_after(Duration::from_millis(if self.hidden { 250 } else { 66 }));
        } else {
            if self.last_refresh.elapsed() > Duration::from_secs(4) && !self.hidden {
                self.refresh_lists();
            }
            let busy = self.status == Status::Measuring || self.files.busy();
            ctx.request_repaint_after(Duration::from_millis(if busy { 100 } else { 1000 }));
        }
        if let Some(t) = self.config_dirty {
            if t.elapsed() > Duration::from_millis(800) {
                self.config_dirty = None;
                self.cfg.save();
            }
        }
        let on_top = if self.cfg.view == ViewMode::Full { self.cfg.always_on_top } else { self.cfg.compact_on_top };
        if self.applied_on_top != Some(on_top) {
            self.applied_on_top = Some(on_top);
            let level = if on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
        let state = TrayState {
            running: self.status == Status::Running,
            isolation: self.cfg.isolation_enabled,
            muted: self.cfg.muted,
            view: self.cfg.view,
            window_visible: !self.hidden,
        };
        if let Some(t) = &mut self.tray {
            t.update(state);
        }
    }

    fn file_engine(&self) -> (vo_dsp::EngineKind, String, vo_dsp::Settings) {
        let (engine, _) = platform::engine_kind(self.cfg.engine);
        let label = match self.cfg.engine {
            EngineChoice::Quality => "Quality engine",
            EngineChoice::Light => "Light engine",
        };
        (engine, label.to_string(), self.cfg.settings())
    }
}

fn pretty(exe: &str) -> String {
    SourceInfo { exe: exe.to_string(), pid: 0, playing: false }.label()
}

impl eframe::App for VoiceolatorApp {
    /// Runs every tick, also while the window is hidden in the tray.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match self.cfg.view {
            ViewMode::Full => self.full_ui(ui),
            ViewMode::Slim => self.slim_ui(ui),
            ViewMode::Pill => self.pill_ui(ui),
        }
        self.close_dialog_ui(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.files.shutdown();
        self.stop();
        self.cfg.save();
    }
}

// ---------- full window ----------

impl VoiceolatorApp {
    fn full_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let running = self.status == Status::Running;
        let measuring = self.status == Status::Measuring;

        egui::Panel::top("header").frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 4 })).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Voiceolator").size(22.0).strong().color(Color32::WHITE));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (text, fill) = if running { ("Stop", STOP) } else { ("Start", ACCENT) };
                    let btn = egui::Button::new(RichText::new(text).size(16.0).strong().color(if running { Color32::WHITE } else { Color32::BLACK }))
                        .fill(fill)
                        .min_size(Vec2::new(108.0, 34.0))
                        .corner_radius(CornerRadius::same(8));
                    if ui.add_enabled(!measuring, btn).on_hover_text("Start or stop listening to the stream").clicked() {
                        self.start_stop();
                    }
                    ui.menu_button(RichText::new("⊟").size(18.0), |ui| {
                        ui.label(RichText::new("Compact window").color(MUTED).size(12.0));
                        if ui.button("Slim bar").clicked() {
                            self.set_view(&ctx, ViewMode::Slim);
                            ui.close();
                        }
                        if ui.button("Tiny pill").clicked() {
                            self.set_view(&ctx, ViewMode::Pill);
                            ui.close();
                        }
                        let mut top = self.cfg.compact_on_top;
                        if ui.checkbox(&mut top, "Keep compact window on top").changed() {
                            self.cfg.compact_on_top = top;
                            self.mark_dirty();
                        }
                        if self.tray_ok() {
                            ui.separator();
                            if ui.button("Hide to tray").clicked() {
                                ui.close();
                                self.hide_window(&ctx);
                            }
                        }
                    })
                    .response
                    .on_hover_text("Compact window / hide to tray");
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (t, label) in [(Tab::Live, "Live stream"), (Tab::Files, "Files")] {
                    let sel = self.tab == t;
                    let b = egui::Button::new(RichText::new(label).color(if sel { Color32::WHITE } else { MUTED }).strong())
                        .fill(if sel { CARD } else { BG })
                        .stroke(if sel { Stroke::new(1.0, ACCENT.linear_multiply(0.6)) } else { Stroke::NONE })
                        .min_size(Vec2::new(110.0, 26.0));
                    if ui.add(b).clicked() {
                        self.tab = t;
                    }
                }
            });
        });

        egui::Panel::bottom("status").frame(egui::Frame::NONE.fill(DARK).inner_margin(egui::Margin::symmetric(14, 8))).show(ui, |ui| {
            let (dot, txt) = match self.status {
                Status::Running => (ACCENT, format!("Running · {}", self.engine_label)),
                Status::Measuring => (WARN, "Measuring latency…".to_string()),
                Status::Stopped => (MUTED, "Stopped".to_string()),
            };
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.5, dot);
                ui.label(RichText::new(txt).color(Color32::LIGHT_GRAY));
                if let Some(f) = self.files.status_line() {
                    ui.label(RichText::new(format!("· {f}")).color(ACCENT2));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(MUTED).size(11.0));
                    if cfg!(windows) {
                        ui.label(RichText::new("Ctrl+Alt+V: on/off · Ctrl+Alt+M: mute").color(MUTED).size(11.0));
                    }
                });
            });
            for m in self.messages.iter().rev().take(2) {
                if m.at.elapsed() > Duration::from_secs(45) && m.level == Level::Info {
                    continue;
                }
                let c = match m.level {
                    Level::Info => Color32::LIGHT_GRAY,
                    Level::Warn => WARN,
                    Level::Error => ERR,
                };
                ui.label(RichText::new(&m.text).color(c).size(12.0));
            }
        });

        egui::CentralPanel::default_margins().frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin::symmetric(14, 6))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Live => {
                    self.audio_card(ui);
                    ui.add_space(8.0);
                    self.isolation_card(ui);
                    ui.add_space(8.0);
                    self.output_card(ui);
                    ui.add_space(8.0);
                    self.monitor_card(ui, running, measuring);
                    ui.add_space(8.0);
                    self.options_card(ui);
                    ui.add_space(6.0);
                }
                Tab::Files => {
                    let mut dirty = false;
                    let (engine, label, settings) = self.file_engine();
                    let mut env = files::Env { cfg: &mut self.cfg, engine, engine_label: label, settings, dirty: &mut dirty };
                    self.files.ui(ui, &mut env);
                    if dirty {
                        self.mark_dirty();
                    }
                    ui.add_space(6.0);
                }
            });
        });

        // drag-and-drop hint
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let r = ctx.content_rect();
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            p.rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(190));
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Drop to isolate the voice", egui::FontId::proportional(22.0), ACCENT);
        }
    }

    fn close_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.close_dialog {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("close_dialog")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.label(RichText::new("Keep Voiceolator running?").size(18.0).strong());
            ui.add_space(4.0);
            ui.label("Voiceolator can keep cleaning the stream in the background. Click its icon in the system tray to bring the window back.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(pill_button("Keep running in the tray", ACCENT, true)).clicked() {
                    choice = Some(true);
                }
                if ui.button("Quit").clicked() {
                    choice = Some(false);
                }
            });
            ui.label(RichText::new("You can change this later in Options.").size(11.5).color(MUTED));
        });
        if let Some(keep) = choice {
            self.close_dialog = false;
            self.cfg.close_asked = true;
            self.cfg.close_to_tray = keep;
            self.cfg.save();
            if keep {
                self.hide_window(ctx);
            } else {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn audio_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "AUDIO ROUTING", |ui| {
            let mut restart = false;
            ui.horizontal(|ui| {
                cell(ui, 70.0, "Listen to");
                let current = self.cfg.source_exe.as_deref().map(pretty).unwrap_or_else(|| "Choose app…".into());
                egui::ComboBox::from_id_salt("source").width(250.0).selected_text(current).show_ui(ui, |ui| {
                    if self.sources.is_empty() {
                        ui.label(RichText::new("No app is playing audio. Start the stream.").color(MUTED));
                    }
                    for s in self.sources.clone() {
                        let sel = self.cfg.source_exe.as_deref().map(|e| e.eq_ignore_ascii_case(&s.exe)).unwrap_or(false);
                        if ui.selectable_label(sel, s.label()).clicked() && !sel {
                            self.cfg.source_exe = Some(s.exe.clone());
                            self.mark_dirty();
                            restart = true;
                        }
                    }
                });
                if ui.button("Refresh").on_hover_text("Refresh apps and devices").clicked() {
                    self.refresh_lists();
                }
            });
            ui.horizontal(|ui| {
                cell(ui, 70.0, "Play on");
                let cur = self.selected_output().name;
                egui::ComboBox::from_id_salt("output").width(250.0).selected_text(cur).show_ui(ui, |ui| {
                    for o in self.outputs.clone() {
                        let sel = o.id == self.cfg.output_id;
                        if ui.selectable_label(sel, &o.name).clicked() && !sel {
                            self.cfg.output_id = o.id.clone();
                            self.cfg.output_name = Some(o.name.clone());
                            self.mark_dirty();
                            restart = true;
                        }
                    }
                });
            });
            let mut mute = self.cfg.hide_original;
            if ui
                .checkbox(&mut mute, "Hide the original stream sound (recommended)")
                .on_hover_text(if cfg!(windows) {
                    "So you don't hear the stream twice: Voiceolator turns the app down while it runs and gives the sound back when you stop. You don't need to change any Windows sound settings."
                } else {
                    "So you don't hear the stream twice: while Voiceolator runs, the app plays into Voiceolator only. Everything goes back when you stop. You don't need to change any sound settings."
                })
                .changed()
            {
                self.cfg.hide_original = mute;
                self.mark_dirty();
                restart = true;
            }
            ui.label(RichText::new("No sound settings to change. Your game's sound is never captured or changed.").size(11.5).color(MUTED));
            if restart {
                self.restart_if_running();
            }
        });
    }

    fn isolation_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "VOICE ISOLATION", |ui| {
            let mut changed = false;
            ui.horizontal(|ui| {
                let on = self.cfg.isolation_enabled;
                let txt = if on { "ON  · voice only" } else { "OFF · original sound" };
                let b = egui::Button::new(RichText::new(txt).strong().color(if on { Color32::BLACK } else { Color32::WHITE }))
                    .fill(if on { ACCENT } else { Color32::from_rgb(60, 64, 82) })
                    .min_size(Vec2::new(170.0, 28.0));
                if ui.add(b).on_hover_text("Switch between the cleaned voice and the original sound (Ctrl+Alt+V)").clicked() {
                    self.cfg.isolation_enabled = !on;
                    changed = true;
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                for p in Preset::ALL {
                    let sel = self.cfg.preset() == p;
                    let b = egui::Button::new(RichText::new(p.label()).color(if sel { Color32::BLACK } else { Color32::LIGHT_GRAY })).fill(if sel { ACCENT2 } else { BUTTON });
                    let hint = match p {
                        Preset::Balanced => "Strong music/effects removal, natural sound",
                        Preset::VoicePriority => "Never cut quiet words; lets a little more background through",
                        _ => "Remove as much background as possible",
                    };
                    if ui.add(b).on_hover_text(hint).clicked() {
                        self.cfg.apply_preset(p);
                        changed = true;
                    }
                }
                if self.cfg.preset() == Preset::Custom {
                    ui.label(RichText::new("Custom").color(ACCENT2));
                }
            });
            ui.add_space(2.0);
            let mut c = self.cfg.clarity;
            let mut s = self.cfg.suppression;
            let mut v = self.cfg.preservation;
            let mut slid = false;
            slid |= slider_row(ui, "Speech clarity", "Brightens the voice and removes rumble", &mut c);
            slid |= slider_row(ui, "Background suppression", "How strongly music and sound effects are removed", &mut s);
            slid |= slider_row(ui, "Voice preservation", "Protects quiet or overlapping speech from being cut", &mut v);
            if slid {
                self.cfg.clarity = c;
                self.cfg.suppression = s;
                self.cfg.preservation = v;
                self.cfg.preset = "custom".into();
                changed = true;
            }
            if changed {
                self.push_settings();
            }
        });
    }

    fn output_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "VOLUME", |ui| {
            ui.horizontal(|ui| {
                let mut db = self.cfg.volume_db;
                ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(80.0);
                let r = ui.add(egui::Slider::new(&mut db, -30.0..=12.0).step_by(0.5).suffix(" dB"));
                if r.changed() {
                    self.cfg.volume_db = db;
                    self.push_settings();
                }
                if r.double_clicked() {
                    self.cfg.volume_db = 0.0;
                    self.push_settings();
                }
                let m = self.cfg.muted;
                let b = egui::Button::new(if m { "Muted" } else { "Mute" }).fill(if m { Color32::from_rgb(90, 50, 55) } else { BUTTON });
                if ui.add(b).on_hover_text("Mute Voiceolator's output (Ctrl+Alt+M)").clicked() {
                    self.toggle_mute();
                }
            });
        });
    }

    fn monitor_card(&mut self, ui: &mut egui::Ui, running: bool, measuring: bool) {
        card(ui, "MONITOR", |ui| {
            let (i, o, sp) = self.meters;
            meter(ui, "Input", db_frac(i), &if running { format!("{:>4.0} dB", i) } else { "  –".into() }, Color32::from_rgb(110, 120, 150));
            meter(ui, "Speech", sp, &if running { format!("{:>4.0} %", sp * 100.0) } else { "  –".into() }, ACCENT);
            meter(ui, "Output", db_frac(o), &if running { format!("{:>4.0} dB", o) } else { "  –".into() }, ACCENT2);
            ui.add_space(2.0);
            let tele = self.host.as_ref().map(|h| h.telemetry());
            ui.horizontal(|ui| {
                cell(ui, 92.0, RichText::new("Latency").color(MUTED));
                match &tele {
                    Some(t) if running && t.engine_ok.load(std::sync::atomic::Ordering::Relaxed) => {
                        let l = t.total_latency_ms();
                        let col = if l < 100.0 { ACCENT } else if l < 150.0 { WARN } else { ERR };
                        ui.label(RichText::new(format!("≈ {l:.0} ms")).strong().color(col)).on_hover_text(format!(
                            "Estimate from the actual buffers: capture {:.0} + queue {:.0} + model {:.0} + compute {:.1} + output {:.0} + device {:.0} ms",
                            t.capture_latency_ms.get(),
                            t.input_queue_ms.get(),
                            t.engine_latency_ms.get(),
                            t.proc_ms_avg.get(),
                            t.output_queue_ms.get(),
                            t.device_queue_ms.get()
                        ));
                    }
                    _ => {
                        ui.label(RichText::new("–").color(MUTED));
                    }
                }
                if let Some(m) = &self.measured {
                    ui.label(RichText::new(format!("measured {m}")).size(11.5).color(MUTED));
                }
            });
            ui.horizontal(|ui| {
                cell(ui, 92.0, RichText::new("CPU").color(MUTED));
                match &tele {
                    Some(t) if running => {
                        ui.label(format!("{:.1} % of one core", t.dsp_load.get() * 100.0));
                        let drops = t.underruns.load(std::sync::atomic::Ordering::Relaxed);
                        if drops > 0 {
                            ui.label(RichText::new(format!("· {drops} dropout(s)")).color(WARN))
                                .on_hover_text("Short gaps because audio arrived late. The buffer grows automatically; if this keeps happening, raise 'Buffer' below.");
                        }
                    }
                    _ => {
                        ui.label(RichText::new("–").color(MUTED));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let b = ui.add_enabled(!measuring && !platform::demo_mode(), egui::Button::new("Measure latency"));
                    if b.on_hover_text("Plays short beeps through the full path and measures the real delay").clicked() {
                        self.start_measurement();
                    }
                });
            });
        });
    }

    fn options_card(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::CollapsingHeader::new(RichText::new("Options").color(MUTED)).default_open(false).show(ui, |ui| {
            let mut restart = false;
            ui.horizontal(|ui| {
                ui.label("Engine");
                let label = |e: EngineChoice| match e {
                    EngineChoice::Quality => "Quality (DPDFNet, best with music)",
                    EngineChoice::Light => "Light (DeepFilterNet3, lower CPU & delay)",
                };
                egui::ComboBox::from_id_salt("engine").width(280.0).selected_text(label(self.cfg.engine)).show_ui(ui, |ui| {
                    for e in [EngineChoice::Quality, EngineChoice::Light] {
                        if ui.selectable_label(self.cfg.engine == e, label(e)).clicked() && self.cfg.engine != e {
                            self.cfg.engine = e;
                            self.mark_dirty();
                            restart = true;
                        }
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label("Buffer");
                let mut b = self.cfg.buffer_ms;
                if ui.add(egui::Slider::new(&mut b, 10.0..=80.0).step_by(5.0).suffix(" ms")).on_hover_text("Larger = more robust against crackles, but more delay").drag_stopped() {
                    restart = true;
                }
                if b != self.cfg.buffer_ms {
                    self.cfg.buffer_ms = b;
                    self.mark_dirty();
                }
            });
            let mut lev = self.cfg.leveler;
            if ui.checkbox(&mut lev, "Auto-level quiet streamers").on_hover_text("Evens out the voice volume. Can also raise kept singing during music-only parts.").changed() {
                self.cfg.leveler = lev;
                self.push_settings();
            }
            let mut auto = self.cfg.start_automatically;
            if ui.checkbox(&mut auto, "Start listening automatically when Voiceolator opens").changed() {
                self.cfg.start_automatically = auto;
                self.mark_dirty();
            }
            ui.add_space(4.0);
            ui.label(RichText::new("Window").color(MUTED).size(12.0));
            let mut top = self.cfg.always_on_top;
            if ui.checkbox(&mut top, "Keep window on top").changed() {
                self.cfg.always_on_top = top;
                self.mark_dirty();
            }
            let mut ctop = self.cfg.compact_on_top;
            if ui.checkbox(&mut ctop, "Keep the slim bar / tiny pill on top").changed() {
                self.cfg.compact_on_top = ctop;
                self.mark_dirty();
            }
            let tray = self.tray_ok();
            ui.add_enabled_ui(tray, |ui| {
                let mut m = self.cfg.minimize_to_tray;
                if ui.checkbox(&mut m, "Minimize to the system tray").changed() {
                    self.cfg.minimize_to_tray = m;
                    self.mark_dirty();
                }
                let mut c = self.cfg.close_to_tray;
                if ui.checkbox(&mut c, "Close button keeps Voiceolator running in the tray").changed() {
                    self.cfg.close_to_tray = c;
                    self.cfg.close_asked = true;
                    self.mark_dirty();
                }
            });
            if !tray {
                let why = if cfg!(windows) {
                    "The system tray is not available."
                } else {
                    "No system tray found on this desktop (on GNOME, enable the \"AppIndicator\" extension)."
                };
                ui.label(RichText::new(why).size(11.5).color(MUTED));
            }
            ui.horizontal(|ui| {
                for v in [ViewMode::Slim, ViewMode::Pill] {
                    if ui.button(format!("Switch to {}", v.label().to_lowercase())).clicked() {
                        self.set_view(&ctx, v);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.link("Open log folder").clicked() {
                    let dir = crate::config::log_path().parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    open_path(&dir);
                }
                if ui.link("Help & troubleshooting").clicked() {
                    open_path(&platform::exe_dir().join("README.html"));
                }
            });
            if restart {
                self.restart_if_running();
            }
        });
    }
}

// ---------- compact layouts ----------

impl VoiceolatorApp {
    /// Background of an undecorated window: drag to move, double-click for the full window.
    fn compact_frame(&mut self, ui: &mut egui::Ui, add: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default_margins().frame(egui::Frame::NONE.fill(CARD).inner_margin(egui::Margin::symmetric(6, 4))).show(ui, |ui| {
            let rect = ui.max_rect().expand(4.0);
            let bg = ui.interact(rect, egui::Id::unique("compact_bg"), egui::Sense::click_and_drag());
            if bg.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if bg.double_clicked() {
                self.set_view(&ctx, ViewMode::Full);
            }
            ui.painter().rect_stroke(ctx.content_rect().shrink(0.5), CornerRadius::same(4), Stroke::new(1.0, ACCENT.linear_multiply(0.35)), egui::StrokeKind::Inside);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                add(self, ui);
            });
        });
    }

    fn power_button(&mut self, ui: &mut egui::Ui, d: f32) {
        let running = self.status == Status::Running;
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(d), egui::Sense::click());
        let fill = if running { ACCENT } else if resp.hovered() { Color32::from_rgb(60, 66, 90) } else { BUTTON };
        let p = ui.painter();
        p.circle_filled(rect.center(), d / 2.0, fill);
        // power symbol
        let c = rect.center();
        let col = if running { Color32::BLACK } else { Color32::WHITE };
        let r = d * 0.26;
        let st = Stroke::new(2.0, col);
        let pts: Vec<egui::Pos2> = (0..=24)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + 0.75 + (2.0 * std::f32::consts::PI - 1.5) * i as f32 / 24.0;
                egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
            })
            .collect();
        p.add(egui::Shape::line(pts, st));
        p.line_segment([egui::pos2(c.x, c.y - r * 1.25), egui::pos2(c.x, c.y - r * 0.2)], st);
        let tip = if running { "Stop listening" } else { "Start listening" };
        if resp.on_hover_text(tip).clicked() && self.status != Status::Measuring {
            self.start_stop();
        }
    }

    fn led(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
        let running = self.status == Status::Running;
        let speech = if running { self.meters.2.clamp(0.0, 1.0) } else { 0.0 };
        let base = if running { Color32::from_rgb(30, 70, 66) } else { Color32::from_rgb(50, 52, 64) };
        let col = lerp(base, ACCENT, speech);
        ui.painter().circle_filled(rect.center(), 5.0, col);
    }

    fn expand_button(&mut self, ui: &mut egui::Ui) {
        if ui.add(egui::Button::new(RichText::new("⛶").size(14.0)).frame(false)).on_hover_text("Full window").clicked() {
            let ctx = ui.ctx().clone();
            self.set_view(&ctx, ViewMode::Full);
        }
    }

    fn slim_ui(&mut self, ui: &mut egui::Ui) {
        self.compact_frame(ui, |s, ui| {
            grip(ui);
            s.power_button(ui, 30.0);
            let running = s.status == Status::Running;
            let on = s.cfg.isolation_enabled;
            let b = pill_button(if on { "Voice only" } else { "Original" }, if on && running { ACCENT } else if on { Color32::from_rgb(40, 96, 90) } else { Color32::from_rgb(60, 64, 82) }, on && running)
                .min_size(Vec2::new(84.0, 24.0));
            if ui.add(b).on_hover_text("Voice isolation on/off (Ctrl+Alt+V)").clicked() {
                s.toggle_isolation();
            }
            let short = match s.cfg.preset() {
                Preset::Balanced => "Balanced",
                Preset::VoicePriority => "Voice+",
                Preset::MaximumSuppression => "Max",
                Preset::Custom => "Custom",
            };
            if ui.add(egui::Button::new(RichText::new(short).color(ACCENT2)).min_size(Vec2::new(72.0, 24.0))).on_hover_text("Preset (click to change)").clicked() {
                s.cycle_preset();
            }
            let m = s.cfg.muted;
            if ui.add(egui::Button::new(if m { "🔇" } else { "🔊" }).frame(false)).on_hover_text("Mute (Ctrl+Alt+M)").clicked() {
                s.toggle_mute();
            }
            ui.spacing_mut().slider_width = 70.0;
            let mut db = s.cfg.volume_db;
            if ui.add(egui::Slider::new(&mut db, -30.0..=12.0).show_value(false)).on_hover_text(format!("Volume {db:+.0} dB")).changed() {
                s.cfg.volume_db = db;
                s.push_settings();
            }
            s.led(ui);
            bar(ui, 46.0, 6.0, if running { db_frac(s.meters.1) } else { 0.0 }, ACCENT2);
            let lat = s.host.as_ref().filter(|_| running).map(|h| h.telemetry().total_latency_ms());
            ui.label(RichText::new(match lat {
                Some(l) => format!("{l:.0} ms"),
                None => "–".into(),
            })
            .monospace()
            .size(11.5)
            .color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                s.hide_button(ui);
                s.expand_button(ui);
            });
        });
    }

    fn pill_ui(&mut self, ui: &mut egui::Ui) {
        self.compact_frame(ui, |s, ui| {
            s.power_button(ui, 28.0);
            s.led(ui);
            let running = s.status == Status::Running;
            let (txt, col) = match (running, s.cfg.isolation_enabled, s.cfg.muted) {
                (false, ..) => ("Off", MUTED),
                (true, _, true) => ("Muted", WARN),
                (true, true, _) => ("Voice", ACCENT),
                (true, false, _) => ("Original", Color32::LIGHT_GRAY),
            };
            let r = ui.add(egui::Button::new(RichText::new(txt).strong().color(col)).frame(false).min_size(Vec2::new(64.0, 22.0)));
            if r.on_hover_text("Click: voice isolation on/off").clicked() {
                s.toggle_isolation();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                s.expand_button(ui);
            });
        });
    }

    fn hide_button(&mut self, ui: &mut egui::Ui) {
        let tip = if self.tray_ok() { "Hide to tray" } else { "Minimize" };
        if ui.add(egui::Button::new(RichText::new("✖").size(12.0)).frame(false)).on_hover_text(tip).clicked() {
            let ctx = ui.ctx().clone();
            self.hide_window(&ctx);
        }
    }
}

/// Six dots: "you can drag this".
fn grip(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 18.0), egui::Sense::hover());
    for i in 0..3 {
        for j in 0..2 {
            let c = egui::pos2(rect.left() + 2.0 + j as f32 * 4.0, rect.top() + 4.0 + i as f32 * 5.0);
            ui.painter().circle_filled(c, 1.2, MUTED);
        }
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}
