//! The Voiceolator window.

use crate::config::{Config, EngineChoice};
use crate::platform::{self, OutputInfo, SourceInfo};
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use vo_audio::{AudioHost, HostConfig, Level};
use vo_dsp::Preset;

const ACCENT: Color32 = Color32::from_rgb(46, 214, 196);
const ACCENT2: Color32 = Color32::from_rgb(160, 132, 255);
const BG: Color32 = Color32::from_rgb(18, 20, 28);
const CARD: Color32 = Color32::from_rgb(28, 31, 43);
const MUTED: Color32 = Color32::from_rgb(140, 146, 168);
const WARN: Color32 = Color32::from_rgb(255, 190, 70);
const ERR: Color32 = Color32::from_rgb(255, 95, 95);

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Stopped,
    Running,
    Measuring,
}

#[allow(dead_code)] // constructed by the Windows hotkey thread
pub enum UiCommand {
    ToggleIsolation,
    ToggleMute,
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
    hotkeys: Option<Receiver<UiCommand>>,
    config_dirty: Option<Instant>,
    applied_on_top: Option<bool>,
}

impl VoiceolatorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, hotkeys: Option<Receiver<UiCommand>>) -> Self {
        style(&cc.egui_ctx);
        let mut cfg = Config::load();
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
            hotkeys,
            config_dirty: None,
            applied_on_top: None,
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

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.hotkeys {
            let cmds: Vec<UiCommand> = rx.try_iter().collect();
            for c in cmds {
                match c {
                    UiCommand::ToggleIsolation => {
                        self.cfg.isolation_enabled = !self.cfg.isolation_enabled;
                        let t = if self.cfg.isolation_enabled { "Voice isolation ON" } else { "Voice isolation OFF (original sound)" };
                        self.say(Level::Info, t);
                    }
                    UiCommand::ToggleMute => self.cfg.muted = !self.cfg.muted,
                }
                self.push_settings();
            }
        }
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
            ctx.request_repaint_after(Duration::from_millis(66));
        } else {
            if self.last_refresh.elapsed() > Duration::from_secs(4) {
                self.refresh_lists();
            }
            ctx.request_repaint_after(Duration::from_millis(if self.status == Status::Measuring { 100 } else { 1000 }));
        }
        if let Some(t) = self.config_dirty {
            if t.elapsed() > Duration::from_millis(800) {
                self.config_dirty = None;
                self.cfg.save();
            }
        }
        if self.applied_on_top != Some(self.cfg.always_on_top) {
            self.applied_on_top = Some(self.cfg.always_on_top);
            let level = if self.cfg.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
        }
    }
}

fn pretty(exe: &str) -> String {
    SourceInfo { exe: exe.to_string(), pid: 0, playing: false }.label()
}

fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.slider_width = 210.0;
        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = BG;
        v.extreme_bg_color = Color32::from_rgb(14, 15, 21);
        v.selection.bg_fill = ACCENT.linear_multiply(0.55);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        v.slider_trailing_fill = true;
        v.override_text_color = Some(Color32::from_rgb(218, 222, 234));
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(6);
        }
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(40, 44, 60);
        v.widgets.inactive.bg_fill = Color32::from_rgb(40, 44, 60);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(52, 57, 78);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(60, 66, 90);
    });
}

fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(12.5).color(MUTED).strong());
            ui.add_space(2.0);
            add(ui)
        })
        .inner
}

/// Horizontal level meter: value in 0..1, with dB/percent caption.
fn meter(ui: &mut egui::Ui, label: &str, frac: f32, caption: &str, color: Color32) {
    ui.horizontal(|ui| {
        cell(ui, 92.0, RichText::new(label).color(MUTED));
        let w = (ui.available_width() - 70.0).max(60.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 10.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(5), Color32::from_rgb(14, 15, 21));
        let mut fill = rect;
        fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
        if fill.width() > 1.0 {
            p.rect_filled(fill, CornerRadius::same(5), color);
        }
        ui.label(RichText::new(caption).monospace().size(11.5).color(MUTED));
    });
}

/// A label in a fixed-width, left-aligned cell (for form-like rows).
fn cell(ui: &mut egui::Ui, w: f32, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.label(text)
    })
    .inner
}

fn db_frac(db: f32) -> f32 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn slider_row(ui: &mut egui::Ui, label: &str, hint: &str, value: &mut f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        cell(ui, 160.0, label).on_hover_text(hint);
        ui.spacing_mut().slider_width = (ui.available_width() - 52.0).max(80.0);
        let r = ui.add(egui::Slider::new(value, 0.0..=1.0).show_value(false));
        changed = r.changed();
        ui.label(RichText::new(format!("{:>3.0}%", *value * 100.0)).monospace().color(MUTED));
    });
    changed
}

impl eframe::App for VoiceolatorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let running = self.status == Status::Running;
        let measuring = self.status == Status::Measuring;

        egui::Panel::top("header").frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin::symmetric(14, 10))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Voiceolator").size(22.0).strong().color(Color32::WHITE));
                ui.label(RichText::new("voice-only stream audio").size(12.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (text, fill) = if running { ("Stop", Color32::from_rgb(110, 48, 60)) } else { ("Start", ACCENT) };
                    let btn = egui::Button::new(RichText::new(text).size(16.0).strong().color(if running { Color32::WHITE } else { Color32::BLACK }))
                        .fill(fill)
                        .min_size(Vec2::new(108.0, 34.0))
                        .corner_radius(CornerRadius::same(8));
                    if ui.add_enabled(!measuring, btn).clicked() {
                        if running {
                            self.stop();
                            self.say(Level::Info, "Stopped. Original audio restored.");
                        } else {
                            self.start();
                        }
                    }
                });
            });
        });

        egui::Panel::bottom("status").frame(egui::Frame::NONE.fill(Color32::from_rgb(14, 15, 21)).inner_margin(egui::Margin::symmetric(14, 8))).show(ui, |ui| {
            let (dot, txt) = match self.status {
                Status::Running => (ACCENT, format!("Running · {}", self.engine_label)),
                Status::Measuring => (WARN, "Measuring latency…".to_string()),
                Status::Stopped => (MUTED, "Stopped".to_string()),
            };
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.5, dot);
                ui.label(RichText::new(txt).color(Color32::LIGHT_GRAY));
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
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                self.audio_card(ui, running);
                ui.add_space(8.0);
                self.isolation_card(ui);
                ui.add_space(8.0);
                self.output_card(ui);
                ui.add_space(8.0);
                self.monitor_card(ui, running, measuring);
                ui.add_space(8.0);
                self.options_card(ui);
                ui.add_space(6.0);
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
        self.cfg.save();
    }
}

impl VoiceolatorApp {
    fn audio_card(&mut self, ui: &mut egui::Ui, _running: bool) {
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
                    let b = egui::Button::new(RichText::new(p.label()).color(if sel { Color32::BLACK } else { Color32::LIGHT_GRAY }))
                        .fill(if sel { ACCENT2 } else { Color32::from_rgb(40, 44, 60) });
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
                let b = egui::Button::new(if m { "Muted" } else { "Mute" }).fill(if m { Color32::from_rgb(90, 50, 55) } else { Color32::from_rgb(40, 44, 60) });
                if ui.add(b).on_hover_text("Mute Voiceolator's output (Ctrl+Alt+M)").clicked() {
                    self.cfg.muted = !m;
                    self.push_settings();
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
                        ui.label(RichText::new(format!("≈ {l:.0} ms")).strong().color(col))
                            .on_hover_text(format!(
                                "Estimate from the actual buffers: capture {:.0} + queue {:.0} + model {:.0} + compute {:.1} + output {:.0} + device {:.0} ms",
                                t.capture_latency_ms.get(), t.input_queue_ms.get(), t.engine_latency_ms.get(), t.proc_ms_avg.get(), t.output_queue_ms.get(), t.device_queue_ms.get()
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
            let mut top = self.cfg.always_on_top;
            if ui.checkbox(&mut top, "Keep window on top").changed() {
                self.cfg.always_on_top = top;
                self.mark_dirty();
            }
            let mut auto = self.cfg.start_automatically;
            if ui.checkbox(&mut auto, "Start listening automatically when Voiceolator opens").changed() {
                self.cfg.start_automatically = auto;
                self.mark_dirty();
            }
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

fn open_path(p: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}
