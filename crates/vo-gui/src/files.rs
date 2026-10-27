//! The "Files" tab: isolate the voice in audio and video files that already exist.

use crate::config::Config;
use crate::theme::*;
use eframe::egui::{self, Color32, CornerRadius, RichText, Vec2};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;
use vo_dsp::{EngineKind, Settings};
use vo_media::ffmpeg::{self, DownloadStage, Ffmpeg};
use vo_media::{AudioFormat, Done, Progress, Stage};

enum ItemState {
    Waiting,
    Working(Option<Progress>),
    Done(Done),
    Failed(String),
    Cancelled,
}

struct Item {
    id: u64,
    path: PathBuf,
    state: ItemState,
}

enum JobEvent {
    Started(u64),
    Progress(u64, Progress),
    Finished(u64, Result<Done, String>),
    Cancelled(u64),
    AllDone,
}

struct Job {
    rx: Receiver<JobEvent>,
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

enum DlEvent {
    Stage(DownloadStage),
    Done(Result<Ffmpeg, String>),
}

struct Download {
    rx: Receiver<DlEvent>,
    cancel: Arc<AtomicBool>,
    stage: Option<DownloadStage>,
    then_start: bool,
}

/// What the app passes in each frame.
pub struct Env<'a> {
    pub cfg: &'a mut Config,
    pub engine: EngineKind,
    pub engine_label: String,
    pub settings: Settings,
    pub dirty: &'a mut bool,
}

pub struct Files {
    items: Vec<Item>,
    next_id: u64,
    job: Option<Job>,
    picker: Option<Receiver<Vec<PathBuf>>>,
    folder_picker: Option<Receiver<Option<PathBuf>>>,
    ffmpeg_picker: Option<Receiver<Option<PathBuf>>>,
    ff: Option<Ffmpeg>,
    ff_search: Option<Receiver<Option<Ffmpeg>>>,
    download: Option<Download>,
    ff_note: Option<(bool, String)>,
    consent: bool,
    started_at: Option<Instant>,
}

fn is_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).map(|e| vo_media::VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl Files {
    pub fn new(ctx: &egui::Context, cfg: &Config) -> Self {
        let mut f = Self {
            items: Vec::new(),
            next_id: 1,
            job: None,
            picker: None,
            folder_picker: None,
            ffmpeg_picker: None,
            ff: None,
            ff_search: None,
            download: None,
            ff_note: None,
            consent: false,
            started_at: None,
        };
        f.search_ffmpeg(ctx, cfg);
        f
    }

    fn search_ffmpeg(&mut self, ctx: &egui::Context, cfg: &Config) {
        let (tx, rx) = channel();
        let chosen = cfg.ffmpeg_path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let f = chosen.and_then(|p| ffmpeg::check(Path::new(&p))).or_else(ffmpeg::find);
            let _ = tx.send(f);
            ctx.request_repaint();
        });
        self.ff_search = Some(rx);
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// One line for the status bar while files are processed.
    pub fn status_line(&self) -> Option<String> {
        self.job.as_ref()?;
        let left = self.items.iter().filter(|i| matches!(i.state, ItemState::Waiting | ItemState::Working(_))).count();
        let cur = self.items.iter().find_map(|i| match &i.state {
            ItemState::Working(Some(p)) => p.fraction,
            _ => None,
        });
        let files = if left == 1 { "1 file".to_string() } else { format!("{left} files") };
        Some(match cur {
            Some(f) => format!("Processing {files} · {:.0}%", f * 100.0),
            None => format!("Processing {files}"),
        })
    }

    pub fn add(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if p.is_dir() {
                // a dropped folder: add the media files directly inside it
                if let Ok(rd) = std::fs::read_dir(&p) {
                    let mut v: Vec<PathBuf> = rd
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().and_then(|e| e.to_str()).map(|e| vo_media::MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false))
                        .collect();
                    v.sort();
                    self.add(v);
                }
                continue;
            }
            if self.items.iter().any(|i| i.path == p && matches!(i.state, ItemState::Waiting | ItemState::Working(_))) {
                continue;
            }
            self.items.push(Item { id: self.next_id, path: p, state: ItemState::Waiting });
            self.next_id += 1;
        }
    }

    /// Items "Isolate voice" will process: new ones, plus cancelled/failed ones (retry).
    fn waiting(&self) -> Vec<(u64, PathBuf)> {
        self.items.iter().filter(|i| matches!(i.state, ItemState::Waiting | ItemState::Cancelled | ItemState::Failed(_))).map(|i| (i.id, i.path.clone())).collect()
    }

    fn needs_ffmpeg(&self, cfg: &Config) -> bool {
        let fmt = AudioFormat::parse(&cfg.files_format).unwrap_or_default();
        fmt.needs_ffmpeg() || (cfg.files_make_video && self.waiting().iter().any(|(_, p)| is_video(p)))
    }

    fn start(&mut self, ctx: &egui::Context, env: &Env) {
        let items = self.waiting();
        if items.is_empty() {
            return;
        }
        for i in &mut self.items {
            if items.iter().any(|(id, _)| *id == i.id) {
                i.state = ItemState::Waiting;
            }
        }
        let cfg = &env.cfg;
        let opts = vo_media::Options {
            engine: env.engine.clone(),
            settings: env.settings,
            out_dir: cfg.files_out_dir.as_ref().map(PathBuf::from),
            audio_format: AudioFormat::parse(&cfg.files_format).unwrap_or_default(),
            make_video: cfg.files_make_video,
            keep_original_track: cfg.files_keep_original,
            ffmpeg: self.ff.as_ref().map(|f| f.path.clone()),
            threads: cfg.files_threads,
        };
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let ctx = ctx.clone();
        let thread = std::thread::Builder::new()
            .name("vo-files".into())
            .spawn(move || {
                for (id, path) in items {
                    if c2.load(Ordering::Relaxed) {
                        let _ = tx.send(JobEvent::Cancelled(id));
                        continue;
                    }
                    let _ = tx.send(JobEvent::Started(id));
                    ctx.request_repaint();
                    let run = |o: &vo_media::Options| {
                        vo_media::process(
                            &path,
                            o,
                            &mut |p| {
                                let _ = tx.send(JobEvent::Progress(id, p));
                                ctx.request_repaint();
                            },
                            &c2,
                        )
                    };
                    let mut r = run(&opts);
                    // The Quality engine needs the VC++ runtime; fall back like the live mode does.
                    if let Err(e) = &r {
                        if matches!(opts.engine, EngineKind::Dpdfnet { .. }) && format!("{e:#}").contains("ONNX Runtime") {
                            log::warn!("Quality engine unavailable for files ({e:#}); using the Light engine");
                            r = run(&vo_media::Options { engine: EngineKind::Dfn3, ..opts.clone() }).map(|mut d| {
                                d.notes.push("Used the Light engine (the Quality engine needs the Microsoft Visual C++ Redistributable).".into());
                                d
                            });
                        }
                    }
                    let ev = match r {
                        Ok(d) => {
                            log::info!("file done: {} -> {}", path.display(), d.output.display());
                            JobEvent::Finished(id, Ok(d))
                        }
                        Err(e) if e.is::<vo_media::Cancelled>() => JobEvent::Cancelled(id),
                        Err(e) => {
                            log::warn!("file failed: {}: {e:#}", path.display());
                            JobEvent::Finished(id, Err(format!("{e:#}")))
                        }
                    };
                    let _ = tx.send(ev);
                    ctx.request_repaint();
                }
                let _ = tx.send(JobEvent::AllDone);
                ctx.request_repaint();
            })
            .ok();
        self.job = Some(Job { rx, cancel, thread });
        self.started_at = Some(Instant::now());
    }

    fn start_download(&mut self, ctx: &egui::Context, then_start: bool) {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = ffmpeg::download(
                &mut |s| {
                    let _ = tx.send(DlEvent::Stage(s));
                    ctx.request_repaint();
                },
                &c2,
            );
            let _ = tx.send(DlEvent::Done(r.map_err(|e| if e.is::<vo_media::Cancelled>() { "cancelled".into() } else { format!("{e:#}") })));
            ctx.request_repaint();
        });
        self.download = Some(Download { rx, cancel, stage: None, then_start });
        self.ff_note = None;
    }

    /// Stop everything (app exit): cancel and wait briefly so no ffmpeg is left running.
    pub fn shutdown(&mut self) {
        if let Some(d) = &self.download {
            d.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(mut j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
            if let Some(t) = j.thread.take() {
                let deadline = Instant::now() + std::time::Duration::from_secs(3);
                while !t.is_finished() && Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
    }

    /// Background events; call every frame (also while hidden).
    pub fn poll(&mut self, ctx: &egui::Context, env: &mut Env) {
        if let Some(rx) = &self.ff_search {
            if let Ok(f) = rx.try_recv() {
                self.ff = f;
                self.ff_search = None;
            }
        }
        if let Some(rx) = &self.picker {
            if let Ok(v) = rx.try_recv() {
                self.picker = None;
                self.add(v);
            }
        }
        if let Some(rx) = &self.folder_picker {
            if let Ok(v) = rx.try_recv() {
                self.folder_picker = None;
                if let Some(d) = v {
                    env.cfg.files_out_dir = Some(d.display().to_string());
                    *env.dirty = true;
                }
            }
        }
        if let Some(rx) = &self.ffmpeg_picker {
            if let Ok(v) = rx.try_recv() {
                self.ffmpeg_picker = None;
                if let Some(p) = v {
                    match ffmpeg::check(&p) {
                        Some(f) => {
                            env.cfg.ffmpeg_path = Some(p.display().to_string());
                            *env.dirty = true;
                            self.ff_note = Some((false, format!("Using ffmpeg {}.", f.version)));
                            self.ff = Some(f);
                        }
                        None => self.ff_note = Some((true, format!("{} is not a working ffmpeg.", file_name(&p)))),
                    }
                }
            }
        }
        let mut start_now = false;
        if let Some(d) = &mut self.download {
            for ev in d.rx.try_iter().collect::<Vec<_>>() {
                match ev {
                    DlEvent::Stage(s) => d.stage = Some(s),
                    DlEvent::Done(r) => {
                        let then = d.then_start;
                        self.download = None;
                        match r {
                            Ok(f) => {
                                self.ff_note = Some((false, format!("ffmpeg {} is ready.", f.version)));
                                self.ff = Some(f);
                                start_now = then;
                            }
                            Err(e) if e == "cancelled" => self.ff_note = Some((false, "Download cancelled.".into())),
                            Err(e) => self.ff_note = Some((true, format!("Could not download ffmpeg: {e}"))),
                        }
                        break;
                    }
                }
            }
        }
        if start_now {
            self.start(ctx, env);
        }
        let mut finished = false;
        if let Some(j) = &self.job {
            for ev in j.rx.try_iter().collect::<Vec<_>>() {
                let set = |items: &mut Vec<Item>, id: u64, st: ItemState| {
                    if let Some(i) = items.iter_mut().find(|i| i.id == id) {
                        i.state = st;
                    }
                };
                match ev {
                    JobEvent::Started(id) => set(&mut self.items, id, ItemState::Working(None)),
                    JobEvent::Progress(id, p) => set(&mut self.items, id, ItemState::Working(Some(p))),
                    JobEvent::Finished(id, Ok(d)) => set(&mut self.items, id, ItemState::Done(d)),
                    JobEvent::Finished(id, Err(e)) => set(&mut self.items, id, ItemState::Failed(e)),
                    JobEvent::Cancelled(id) => set(&mut self.items, id, ItemState::Cancelled),
                    JobEvent::AllDone => finished = true,
                }
            }
        }
        if finished {
            if let Some(mut j) = self.job.take() {
                if let Some(t) = j.thread.take() {
                    let _ = t.join();
                }
            }
            for i in &mut self.items {
                if matches!(i.state, ItemState::Working(_)) {
                    i.state = ItemState::Cancelled;
                }
            }
        }
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = rfd::FileDialog::new()
                .set_title("Choose audio or video files")
                .add_filter("Audio and video", vo_media::MEDIA_EXTENSIONS)
                .add_filter("All files", &["*"])
                .pick_files()
                .unwrap_or_default();
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.picker = Some(rx);
    }

    fn pick_folder(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(rfd::FileDialog::new().set_title("Where should the clean files go?").pick_folder());
            ctx.request_repaint();
        });
        self.folder_picker = Some(rx);
    }

    fn pick_ffmpeg(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new().set_title("Choose ffmpeg");
            if cfg!(windows) {
                d = d.add_filter("ffmpeg.exe", &["exe"]);
            }
            let _ = tx.send(d.pick_file());
            ctx.request_repaint();
        });
        self.ffmpeg_picker = Some(rx);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, env: &mut Env) {
        let ctx = ui.ctx().clone();
        card(ui, "FILES", |ui| self.list_ui(ui, &ctx, env));
        ui.add_space(8.0);
        card(ui, "OUTPUT", |ui| self.output_ui(ui, &ctx, env));
        ui.add_space(8.0);
        card(ui, "FFMPEG", |ui| self.ffmpeg_ui(ui, &ctx, env));
        self.consent_ui(&ctx, env);
    }

    fn list_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, env: &mut Env) {
        if self.items.is_empty() {
            egui::Frame::NONE.fill(DARK).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(12, 18)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Drop audio or video files here").size(16.0).strong());
                    ui.label(RichText::new("Twitch VODs, recordings, podcasts… MP4, MKV, WEBM, MOV, MP3, M4A, WAV, FLAC, OGG").size(11.5).color(MUTED));
                    ui.add_space(6.0);
                    if ui.add_enabled(self.picker.is_none(), pill_button("Choose files…", ACCENT, true).min_size(Vec2::new(140.0, 30.0))).clicked() {
                        self.pick_files(ctx);
                    }
                });
            });
            ui.label(RichText::new("You get \"name (voice).ext\" next to each file. The originals are never changed.").size(11.5).color(MUTED));
            return;
        }
        let mut remove = None;
        let busy = self.job.is_some();
        egui::ScrollArea::vertical().id_salt("files_list").max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
            for it in &self.items {
                ui.horizontal(|ui| {
                    // right part first, so the file name gets (and is truncated to) the rest
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_remove = !matches!(it.state, ItemState::Working(_));
                        if can_remove && ui.small_button("✖").on_hover_text("Remove from the list").clicked() {
                            remove = Some(it.id);
                        }
                        match &it.state {
                            ItemState::Waiting => {
                                ui.label(RichText::new(if busy { "waiting" } else { "ready" }).color(MUTED).size(12.0));
                            }
                            ItemState::Working(p) => {
                                let (frac, txt) = match p {
                                    Some(Progress { stage: Stage::Processing, fraction, speed, seconds_done }) => {
                                        let eta = match (fraction, speed) {
                                            (Some(f), Some(s)) if *f > 0.01 && *s > 0.0 => {
                                                let total = seconds_done / *f as f64;
                                                format!(" · {} left", fmt_time((total - seconds_done) / *s as f64))
                                            }
                                            _ => String::new(),
                                        };
                                        (fraction.unwrap_or(0.0), format!("{:.0}% · {:.0}× speed{eta}", fraction.unwrap_or(0.0) * 100.0, speed.unwrap_or(0.0)))
                                    }
                                    Some(Progress { stage: Stage::Finishing, .. }) => (1.0, "finishing…".into()),
                                    _ => (0.0, "opening…".into()),
                                };
                                ui.label(RichText::new(txt).color(MUTED).size(12.0));
                                bar(ui, 70.0, 8.0, frac, ACCENT);
                            }
                            ItemState::Done(d) => {
                                if ui.small_button("Show").on_hover_text("Show in folder").clicked() {
                                    reveal(&d.output);
                                }
                                if ui.small_button("Play").on_hover_text(format!("Open {}", file_name(&d.output))).clicked() {
                                    open_path(&d.output);
                                }
                                let mut tip = format!("Saved as {}\n{:.0} s of audio in {:.0} s with the {}", d.output.display(), d.audio_seconds, d.elapsed_s, d.engine);
                                for n in &d.notes {
                                    tip.push_str(&format!("\n⚠ {n}"));
                                }
                                let (txt, col) = if d.notes.is_empty() { (if d.video { "video saved" } else { "audio saved" }, ACCENT) } else { ("saved ⚠", WARN) };
                                ui.label(RichText::new(txt).color(col).size(12.0)).on_hover_text(tip);
                            }
                            ItemState::Failed(e) => {
                                ui.add(egui::Label::new(RichText::new(format!("failed: {e}")).color(ERR).size(12.0)).truncate()).on_hover_text(e);
                            }
                            ItemState::Cancelled => {
                                ui.label(RichText::new("cancelled").color(MUTED).size(12.0));
                            }
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let (icon, col) = match &it.state {
                                ItemState::Waiting => ("•", MUTED),
                                ItemState::Working(_) => ("▶", ACCENT),
                                ItemState::Done(_) => ("✔", ACCENT),
                                ItemState::Failed(_) => ("✖", ERR),
                                ItemState::Cancelled => ("–", MUTED),
                            };
                            ui.label(RichText::new(icon).color(col));
                            ui.add(egui::Label::new(file_name(&it.path)).truncate()).on_hover_text(it.path.display().to_string());
                        });
                    });
                });
            }
        });
        if let Some(id) = remove {
            self.items.retain(|i| i.id != id);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.picker.is_none(), egui::Button::new("Add files…")).clicked() {
                self.pick_files(ctx);
            }
            let finished = self.items.iter().any(|i| matches!(i.state, ItemState::Done(_) | ItemState::Failed(_) | ItemState::Cancelled));
            if ui.add_enabled(finished, egui::Button::new("Clear finished")).clicked() {
                self.items.retain(|i| matches!(i.state, ItemState::Waiting | ItemState::Working(_)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(j) = &self.job {
                    if ui.add(pill_button("Cancel", STOP, false).min_size(Vec2::new(120.0, 30.0))).clicked() {
                        j.cancel.store(true, Ordering::Relaxed);
                    }
                } else {
                    let n = self.waiting().len();
                    let label = if n <= 1 { "Isolate voice".to_string() } else { format!("Isolate voice ({n} files)") };
                    let b = pill_button(label, ACCENT, true).min_size(Vec2::new(150.0, 30.0));
                    if ui.add_enabled(n > 0 && self.download.is_none(), b).clicked() {
                        if self.ff.is_none() && self.needs_ffmpeg(env.cfg) && ffmpeg::DOWNLOAD.is_some() {
                            self.consent = true;
                        } else {
                            self.start(ctx, env);
                        }
                    }
                }
            });
        });
    }

    fn output_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, env: &mut Env) {
        let has_ff = self.ff.is_some();
        let cfg = &mut *env.cfg;
        let mut dirty = false;
        ui.horizontal(|ui| {
            cell(ui, 92.0, "Save to");
            let mut next_to = cfg.files_out_dir.is_none();
            if ui.radio_value(&mut next_to, true, "Next to the original").clicked() {
                cfg.files_out_dir = None;
                dirty = true;
            }
            if ui.radio_value(&mut next_to, false, "Folder").clicked() && cfg.files_out_dir.is_none() {
                self.pick_folder(ctx);
            }
        });
        if let Some(d) = cfg.files_out_dir.clone() {
            ui.horizontal(|ui| {
                cell(ui, 92.0, "");
                let short = if d.chars().count() > 40 { format!("…{}", d.chars().rev().take(39).collect::<Vec<_>>().into_iter().rev().collect::<String>()) } else { d.clone() };
                ui.label(RichText::new(short).color(MUTED).size(12.0)).on_hover_text(&d);
                if ui.small_button("Change…").clicked() {
                    self.pick_folder(ctx);
                }
            });
        }
        ui.horizontal(|ui| {
            cell(ui, 92.0, "Audio as");
            let cur = AudioFormat::parse(&cfg.files_format).unwrap_or_default();
            egui::ComboBox::from_id_salt("files_fmt").width(250.0).selected_text(cur.label()).show_ui(ui, |ui| {
                for f in AudioFormat::ALL {
                    let label = if f.needs_ffmpeg() && !has_ff { format!("{} – needs ffmpeg", f.label()) } else { f.label().to_string() };
                    if ui.selectable_label(cur == f, label).clicked() {
                        cfg.files_format = f.ext().into();
                        dirty = true;
                    }
                }
            });
        });
        if ui
            .checkbox(&mut cfg.files_make_video, "For videos: save a video with the clean voice")
            .on_hover_text("The picture is copied unchanged (no quality loss) and the sound is replaced by the clean voice. Needs ffmpeg. Without it, the clean voice is saved as audio.")
            .changed()
        {
            dirty = true;
        }
        ui.add_enabled_ui(cfg.files_make_video, |ui| {
            if ui
                .checkbox(&mut cfg.files_keep_original, "Keep the original sound as a second audio track")
                .on_hover_text("Lets you switch between the clean voice and the original in your video player.")
                .changed()
            {
                dirty = true;
            }
        });
        ui.horizontal(|ui| {
            cell(ui, 92.0, "CPU cores");
            let max = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).max(1);
            let mut auto = cfg.files_threads == 0;
            if ui.checkbox(&mut auto, "Auto").on_hover_text(format!("Auto uses {} of {max} cores, leaving the rest for your game.", vo_media::default_threads())).changed() {
                cfg.files_threads = if auto { 0 } else { vo_media::default_threads() };
                dirty = true;
            }
            if !auto {
                let mut n = cfg.files_threads.clamp(1, max);
                ui.spacing_mut().slider_width = 120.0;
                if ui.add(egui::Slider::new(&mut n, 1..=max)).on_hover_text("More cores = faster, but less CPU left for other programs").changed() {
                    cfg.files_threads = n;
                    dirty = true;
                }
            }
        });
        ui.label(RichText::new(format!("Uses the voice settings from the Live tab: {} preset, {}.", env.settings.preset.label(), env.engine_label)).size(11.5).color(MUTED));
        if dirty {
            *env.dirty = true;
        }
    }

    fn ffmpeg_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, env: &mut Env) {
        if let Some(d) = &self.download {
            ui.horizontal(|ui| {
                let (frac, txt) = match d.stage {
                    Some(DownloadStage::Downloading { done, total }) => (done as f32 / total.max(1) as f32, format!("Downloading ffmpeg… {:.0} of {:.0} MB", done as f64 / 1e6, total as f64 / 1e6)),
                    Some(DownloadStage::Extracting) => (1.0, "Unpacking…".to_string()),
                    None => (0.0, "Connecting…".to_string()),
                };
                ui.label(txt);
                bar(ui, 120.0, 8.0, frac, ACCENT);
                if ui.small_button("Cancel").clicked() {
                    d.cancel.store(true, Ordering::Relaxed);
                }
            });
        } else if self.ff_search.is_some() {
            ui.label(RichText::new("Looking for ffmpeg…").color(MUTED));
        } else if let Some(f) = self.ff.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("✔").color(ACCENT));
                ui.label(format!("ffmpeg {}", f.version));
                ui.add(egui::Label::new(RichText::new(f.path.display().to_string()).size(11.5).color(MUTED)).truncate()).on_hover_text(f.path.display().to_string());
            });
            ui.horizontal(|ui| {
                if ui.small_button("Use another…").clicked() {
                    self.pick_ffmpeg(ctx);
                }
                if f.path.starts_with(ffmpeg::managed_dir()) && ui.small_button("Remove downloaded ffmpeg").clicked() {
                    match ffmpeg::remove_downloaded() {
                        Ok(()) => {
                            self.ff = None;
                            self.search_ffmpeg(ctx, env.cfg);
                        }
                        Err(e) => self.ff_note = Some((true, format!("Could not remove it: {e}"))),
                    }
                }
                if env.cfg.ffmpeg_path.is_some() && ui.small_button("Search automatically").clicked() {
                    env.cfg.ffmpeg_path = None;
                    *env.dirty = true;
                    self.search_ffmpeg(ctx, env.cfg);
                }
            });
        } else {
            ui.label("ffmpeg is not installed. It is needed for videos and for MP3/M4A/FLAC. WAV audio works without it.");
            ui.horizontal(|ui| {
                if let Some(info) = &ffmpeg::DOWNLOAD {
                    if ui.add(pill_button(format!("Download ffmpeg ({:.0} MB)", info.size as f64 / 1e6), BUTTON, false)).on_hover_text(download_details()).clicked() {
                        self.start_download(ctx, false);
                    }
                }
                if ui.button("I have ffmpeg…").on_hover_text("Choose an ffmpeg program you already have").clicked() {
                    self.pick_ffmpeg(ctx);
                }
            });
        }
        if let Some((err, t)) = &self.ff_note {
            ui.label(RichText::new(t).size(12.0).color(if *err { ERR } else { MUTED }));
        }
    }

    fn consent_ui(&mut self, ctx: &egui::Context, env: &mut Env) {
        if !self.consent {
            return;
        }
        let Some(info) = &ffmpeg::DOWNLOAD else {
            self.consent = false;
            return;
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("ffmpeg_consent")).show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.label(RichText::new("Download ffmpeg?").size(18.0).strong());
            ui.add_space(4.0);
            ui.label("Videos and MP3 / M4A / FLAC files need ffmpeg, a free program for reading and writing media files. Voiceolator can download it for you:");
            ui.add_space(2.0);
            for l in [
                format!("ffmpeg {} ({} build, {} licence), {:.0} MB", info.version, info.publisher, info.license, info.size as f64 / 1e6),
                format!("from {}", host(info.urls[0])),
                format!("saved in {}", ffmpeg::managed_dir().display()),
                "checked against a fixed checksum before it is used".into(),
            ] {
                ui.label(RichText::new(format!("•  {l}")).size(12.5).color(Color32::LIGHT_GRAY));
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Without ffmpeg, the clean voice is saved as WAV audio.").size(12.0).color(MUTED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(pill_button("Download and continue", ACCENT, true)).clicked() {
                    choice = Some(1);
                }
                if ui.button("Continue without").clicked() {
                    choice = Some(2);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
            });
        });
        match choice {
            Some(1) => {
                self.consent = false;
                self.start_download(ctx, true);
            }
            Some(2) => {
                self.consent = false;
                self.start(ctx, env);
            }
            Some(_) => self.consent = false,
            None => {}
        }
    }
}

fn host(url: &str) -> String {
    url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url).to_string()
}

fn download_details() -> String {
    match &ffmpeg::DOWNLOAD {
        Some(i) => format!(
            "ffmpeg {} ({} build, {}) from {}\nSaved in {}\nVerified with a fixed SHA-256 checksum.",
            i.version,
            i.publisher,
            i.license,
            host(i.urls[0]),
            ffmpeg::managed_dir().display()
        ),
        None => String::new(),
    }
}
