//! The audio host: three threads connected by lock-free ring buffers.
//!
//! ```text
//!  capture thread           DSP thread                    render thread
//!  (OS input stream)  ──►  ring A  ──►  Pipeline (model)  ──►  ring B  ──►  drift resampler ──► (OS output)
//!   downmix to mono         48 kHz mono, 10 ms hops                  keeps ring B at target fill
//! ```
//!
//! * Each I/O thread owns its OS stream and recreates it with back-off when it fails
//!   (device unplugged, app closed, default device changed), so the host survives
//!   disconnects without user action.
//! * The DSP thread owns the (non-`Send`) model. If the preferred engine cannot be loaded
//!   it falls back to DeepFilterNet3 and reports why.
//! * Nothing here touches any audio other than the selected source; the game's own audio
//!   path is never opened.

use crate::backend::{InputFactory, OutputFactory};
use crate::telemetry::Telemetry;
use anyhow::Result;
use parking_lot::Mutex;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use vo_dsp::drift::DriftResampler;
use vo_dsp::{create_engine, EngineKind, Pipeline, Settings};

pub const SR: u32 = 48000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct HostEvent {
    pub level: Level,
    pub message: String,
}

pub struct HostConfig {
    pub input: InputFactory,
    pub output: OutputFactory,
    pub engine: EngineKind,
    pub settings: Settings,
    /// Jitter buffer between DSP and output (ms). 20 ms is safe on most systems.
    pub output_buffer_ms: f32,
    /// Optional A/B recording: (original input, processed output) as 48 kHz mono WAVs.
    pub record: Option<(PathBuf, PathBuf)>,
}

/// Detects the onsets of test bursts in the output stream (latency measurement).
#[derive(Default)]
pub struct Probe {
    env: f32,
    quiet: usize,
    /// Play times (100 ns performance-counter units) of detected onsets.
    pub onsets: Vec<i64>,
}

impl Probe {
    const THRESH: f32 = 0.04;
    /// Returns Some(()) at a burst onset: level crosses the threshold after >= 250 ms quiet.
    pub fn feed(&mut self, x: f32) -> Option<()> {
        let a = x.abs();
        self.env = if a > self.env { a } else { self.env * 0.999 };
        if self.env > Self::THRESH {
            let onset = self.quiet >= (SR as usize / 4);
            self.quiet = 0;
            if onset {
                return Some(());
            }
        } else if self.env < Self::THRESH * 0.5 {
            self.quiet += 1;
        }
        None
    }
}

struct Shared {
    tele: Arc<Telemetry>,
    probe: Mutex<Option<Probe>>,
    /// The DSP thread, woken by the capture thread whenever new audio arrives.
    dsp_waker: Mutex<Option<thread::Thread>>,
    stop: AtomicBool,
    settings: Mutex<Settings>,
    settings_ver: AtomicU64,
    events: Mutex<Sender<HostEvent>>,
}

impl Shared {
    fn emit(&self, level: Level, message: impl Into<String>) {
        let message = message.into();
        match level {
            Level::Info => log::info!("{message}"),
            Level::Warn => log::warn!("{message}"),
            Level::Error => log::error!("{message}"),
        }
        let _ = self.events.lock().send(HostEvent { level, message });
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

pub struct AudioHost {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    events: Receiver<HostEvent>,
}

type Prod = <HeapRb<f32> as Split>::Prod;
type Cons = <HeapRb<f32> as Split>::Cons;

impl AudioHost {
    pub fn start(cfg: HostConfig) -> Result<Self> {
        let (tx, rx) = channel();
        let tele = Arc::new(Telemetry::default());
        tele.running.store(true, Ordering::Relaxed);
        let shared = Arc::new(Shared {
            tele,
            dsp_waker: Mutex::new(None),
            probe: Mutex::new(None),
            stop: AtomicBool::new(false),
            settings: Mutex::new(cfg.settings),
            settings_ver: AtomicU64::new(1),
            events: Mutex::new(tx),
        });
        // 2 s of headroom each; normal fill is a few tens of ms.
        let (a_prod, a_cons) = HeapRb::<f32>::new(SR as usize * 2).split();
        let (b_prod, b_cons) = HeapRb::<f32>::new(SR as usize * 2).split();

        let mut threads = Vec::new();
        let s = shared.clone();
        let input = cfg.input;
        threads.push(spawn("vo-capture", move || capture_thread(s, input, a_prod))?);

        let s = shared.clone();
        let engine = cfg.engine;
        let record = cfg.record;
        let target_ms = cfg.output_buffer_ms;
        threads.push(spawn("vo-dsp", move || dsp_thread(s, engine, a_cons, b_prod, record))?);

        let s = shared.clone();
        let output = cfg.output;
        threads.push(spawn("vo-render", move || render_thread(s, output, b_cons, target_ms))?);

        Ok(Self { shared, threads, events: rx })
    }

    pub fn telemetry(&self) -> Arc<Telemetry> {
        self.shared.tele.clone()
    }

    pub fn settings(&self) -> Settings {
        *self.shared.settings.lock()
    }

    /// Apply new settings; picked up by the DSP thread within one hop.
    pub fn set_settings(&self, s: Settings) {
        *self.shared.settings.lock() = s;
        self.shared.settings_ver.fetch_add(1, Ordering::Release);
    }

    /// Start detecting test-burst onsets in the output (see `windows::latency`).
    pub fn start_probe(&self) {
        *self.shared.probe.lock() = Some(Probe::default());
    }

    /// Stop the probe and return the play times of detected onsets (100 ns QPC units).
    pub fn take_probe(&self) -> Vec<i64> {
        self.shared.probe.lock().take().map(|p| p.onsets).unwrap_or_default()
    }

    /// Drain pending status events (non-blocking).
    pub fn poll_events(&self) -> Vec<HostEvent> {
        self.events.try_iter().collect()
    }

    pub fn is_finished(&self) -> bool {
        self.threads.iter().all(|t| t.is_finished())
    }

    /// Stop all threads and wait for them (graceful shutdown).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        self.shared.tele.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for AudioHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>> {
    Ok(thread::Builder::new().name(name.into()).spawn(f)?)
}

/// Sleep in small steps so that stop requests are honoured quickly.
fn backoff_sleep(shared: &Shared, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until && !shared.stopped() {
        thread::sleep(Duration::from_millis(20));
    }
}

fn capture_thread(shared: Arc<Shared>, factory: InputFactory, mut prod: Prod) {
    crate::rt::promote_current_thread();
    let tele = &shared.tele;
    let mut backoff = 250u64;
    let mut interleaved: Vec<f32> = Vec::with_capacity(SR as usize);
    let mut mono: Vec<f32> = Vec::with_capacity(SR as usize);
    let mut first = true;
    while !shared.stopped() {
        let mut stream = match factory() {
            Ok(s) => s,
            Err(e) => {
                tele.capture_ok.store(false, Ordering::Relaxed);
                shared.emit(Level::Error, format!("Audio source unavailable: {e:#}. Retrying…"));
                backoff_sleep(&shared, backoff);
                backoff = (backoff * 2).min(5000);
                continue;
            }
        };
        if stream.sample_rate() != SR {
            shared.emit(Level::Error, format!("Capture stream runs at {} Hz; 48000 Hz required", stream.sample_rate()));
            backoff_sleep(&shared, 2000);
            continue;
        }
        if !first {
            tele.restarts.fetch_add(1, Ordering::Relaxed);
        }
        first = false;
        tele.capture_ok.store(true, Ordering::Relaxed);
        tele.capture_latency_ms.set(stream.latency_frames() as f32 * 1000.0 / SR as f32);
        shared.emit(Level::Info, "Capturing audio");
        backoff = 250;
        let ch = stream.channels().max(1);
        loop {
            if shared.stopped() {
                return;
            }
            interleaved.clear();
            match stream.read(&mut interleaved, 100) {
                Ok(0) => continue,
                Ok(_) => {
                    mono.clear();
                    let inv = 1.0 / ch as f32;
                    mono.extend(interleaved.chunks_exact(ch).map(|f| f.iter().sum::<f32>() * inv));
                    let pushed = prod.push_slice(&mono);
                    if pushed < mono.len() {
                        tele.overruns.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(t) = shared.dsp_waker.lock().as_ref() {
                        t.unpark();
                    }
                }
                Err(e) => {
                    tele.capture_ok.store(false, Ordering::Relaxed);
                    shared.emit(Level::Warn, format!("Capture interrupted: {e:#}. Reconnecting…"));
                    break;
                }
            }
        }
        backoff_sleep(&shared, backoff);
    }
}

struct Recorder {
    tx: Sender<(Vec<f32>, Vec<f32>)>,
    handle: Option<JoinHandle<()>>,
}

impl Recorder {
    fn start(shared: Arc<Shared>, paths: (PathBuf, PathBuf)) -> Option<Self> {
        let spec = hound::WavSpec { channels: 1, sample_rate: SR, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let wi = hound::WavWriter::create(&paths.0, spec);
        let wo = hound::WavWriter::create(&paths.1, spec);
        let (mut wi, mut wo) = match (wi, wo) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => {
                shared.emit(Level::Error, format!("Cannot create recording files: {e}"));
                return None;
            }
        };
        let (tx, rx) = channel::<(Vec<f32>, Vec<f32>)>();
        let handle = thread::Builder::new()
            .name("vo-record".into())
            .spawn(move || {
                for (i, o) in rx {
                    for s in i {
                        let _ = wi.write_sample(s);
                    }
                    for s in o {
                        let _ = wo.write_sample(s);
                    }
                }
                let _ = wi.finalize();
                let _ = wo.finalize();
            })
            .ok();
        Some(Self { tx, handle })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let (tx, _) = channel();
        drop(std::mem::replace(&mut self.tx, tx));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn dsp_thread(shared: Arc<Shared>, kind: EngineKind, mut cons: Cons, mut prod: Prod, record: Option<(PathBuf, PathBuf)>) {
    let tele = &shared.tele;
    crate::rt::promote_current_thread();
    *shared.dsp_waker.lock() = Some(thread::current());
    let mut ver = shared.settings_ver.load(Ordering::Acquire);
    let settings = *shared.settings.lock();

    let engine = match create_engine(&kind, &settings) {
        Ok(e) => e,
        Err(e) if kind != EngineKind::Dfn3 => {
            shared.emit(Level::Warn, format!("{} unavailable ({e:#}); falling back to DeepFilterNet3", kind.label()));
            match create_engine(&EngineKind::Dfn3, &settings) {
                Ok(e) => e,
                Err(e) => {
                    shared.emit(Level::Error, format!("Voice engine failed to start: {e:#}"));
                    return;
                }
            }
        }
        Err(e) => {
            shared.emit(Level::Error, format!("Voice engine failed to start: {e:#}"));
            return;
        }
    };
    let mut pipe = Pipeline::new(engine, settings);
    tele.engine_ok.store(true, Ordering::Relaxed);
    shared.emit(Level::Info, format!("Voice engine: {}", pipe.engine_name()));
    let hop = pipe.hop_size();
    let hop_ms = hop as f32 * 1000.0 / SR as f32;
    tele.engine_latency_ms.set((pipe.latency_samples() + hop) as f32 * 1000.0 / SR as f32);

    let recorder = record.and_then(|p| Recorder::start(shared.clone(), p));
    let mut inp = vec![0.0f32; hop];
    let mut out = vec![0.0f32; hop];
    let (mut proc_avg, mut proc_max, mut max_decay) = (0.0f32, 0.0f32, 0u32);
    let mut err_reported = false;
    // Never let more than this much audio pile up before the model (e.g. after a stall).
    let max_backlog = (SR as usize / 1000) * 120;

    while !shared.stopped() {
        let v = shared.settings_ver.load(Ordering::Acquire);
        if v != ver {
            ver = v;
            pipe.apply_settings(*shared.settings.lock());
        }
        let avail = cons.occupied_len();
        if avail > max_backlog {
            // Catch up instead of running permanently late.
            cons.skip(avail - hop);
            tele.overruns.fetch_add(1, Ordering::Relaxed);
        }
        if cons.occupied_len() < hop {
            // Woken by the capture thread; the timeout only guards against lost wake-ups.
            thread::park_timeout(Duration::from_millis(5));
            continue;
        }
        cons.pop_slice(&mut inp);
        tele.input_queue_ms.set(cons.occupied_len() as f32 * 1000.0 / SR as f32);

        let t0 = Instant::now();
        match pipe.process_hop(&inp, &mut out) {
            Ok(st) => {
                tele.input_peak_db.set(st.input_peak_db);
                tele.output_peak_db.set(st.output_peak_db);
                tele.speech_activity.set(st.speech_activity);
                err_reported = false;
            }
            Err(e) => {
                out.fill(0.0);
                if !err_reported {
                    shared.emit(Level::Error, format!("Processing error: {e:#}"));
                    err_reported = true;
                }
            }
        }
        let dt = t0.elapsed().as_secs_f32() * 1000.0;
        proc_avg = 0.98 * proc_avg + 0.02 * dt;
        max_decay += 1;
        if dt > proc_max || max_decay > 500 {
            proc_max = dt;
            max_decay = 0;
        }
        tele.proc_ms_avg.set(proc_avg);
        tele.proc_ms_max.set(proc_max);
        tele.dsp_load.set(proc_avg / hop_ms);
        tele.hops_processed.fetch_add(1, Ordering::Relaxed);

        if prod.push_slice(&out) < hop {
            tele.overruns.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(r) = &recorder {
            let _ = r.tx.send((inp.clone(), out.clone()));
        }
    }
    drop(recorder);
}

fn render_thread(shared: Arc<Shared>, factory: OutputFactory, mut cons: Cons, target_ms: f32) {
    crate::rt::promote_current_thread();
    let tele = &shared.tele;
    let mut backoff = 250u64;
    let mut mono: Vec<f32> = vec![0.0; SR as usize];
    let mut inter: Vec<f32> = Vec::with_capacity(SR as usize * 2);
    let base_target = ((target_ms.max(5.0) / 1000.0) * SR as f32) as usize;
    // Adaptive jitter buffer: grows by 5 ms after each dropout (up to 100 ms) and shrinks
    // back towards the configured value after 30 s without dropouts.
    let mut target = base_target;
    let max_target = SR as usize / 10;
    let mut last_glitch = Instant::now();
    let mut first = true;
    while !shared.stopped() {
        let mut stream = match factory() {
            Ok(s) => s,
            Err(e) => {
                tele.output_ok.store(false, Ordering::Relaxed);
                shared.emit(Level::Error, format!("Output device unavailable: {e:#}. Retrying…"));
                backoff_sleep(&shared, backoff);
                backoff = (backoff * 2).min(5000);
                continue;
            }
        };
        if stream.sample_rate() != SR {
            shared.emit(Level::Error, format!("Output stream runs at {} Hz; 48000 Hz required", stream.sample_rate()));
            backoff_sleep(&shared, 2000);
            continue;
        }
        if !first {
            tele.restarts.fetch_add(1, Ordering::Relaxed);
        }
        first = false;
        tele.output_ok.store(true, Ordering::Relaxed);
        shared.emit(Level::Info, "Output started");
        backoff = 250;
        let ch = stream.channels().max(1);
        let mut drift = DriftResampler::new(SR as usize, target);
        // Start with exactly the target fill so latency is deterministic.
        let excess = cons.occupied_len().saturating_sub(target);
        cons.skip(excess);
        let mut starving_since: Option<Instant> = None;
        let mut frames_written: u64 = 0;
        // Don't treat the start-up period (before the first audio arrives) as dropouts.
        let mut primed = false;

        let res: Result<()> = (|| loop {
            if shared.stopped() {
                return Ok(());
            }
            stream.wait(200)?;
            let n = stream.writable_frames()?;
            if n == 0 {
                continue;
            }
            let fill = cons.occupied_len();
            // Hard resync if we are far off target (e.g. after a capture gap).
            if fill > target + SR as usize / 5 {
                cons.skip(fill - target);
                tele.overruns.fetch_add(1, Ordering::Relaxed);
            }
            primed |= cons.occupied_len() >= target;
            drift.update(cons.occupied_len(), n);
            let starved = drift.produce(&mut mono[..n], || cons.try_pop());
            if starved > 0 && primed {
                // Buffer ran dry: a capture hiccup, or the source stopped sending.
                if starving_since.is_none() {
                    starving_since = Some(Instant::now());
                    tele.underruns.fetch_add(1, Ordering::Relaxed);
                    target = (target + SR as usize / 200).min(max_target);
                    drift.set_target_fill(target);
                    last_glitch = Instant::now();
                }
            } else {
                if let Some(t) = starving_since.take() {
                    if t.elapsed() > Duration::from_millis(150) {
                        // A long gap means the source paused (e.g. stream paused, app silent),
                        // not a timing glitch: undo the jitter-buffer increase.
                        target = target.saturating_sub(SR as usize / 200).max(base_target);
                        drift.set_target_fill(target);
                        tele.underruns.fetch_sub(1, Ordering::Relaxed);
                        tele.source_gaps.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if target > base_target && last_glitch.elapsed() > Duration::from_secs(30) {
                    target = (target - SR as usize / 1000).max(base_target);
                    drift.set_target_fill(target);
                    last_glitch = Instant::now() - Duration::from_secs(25);
                }
            }
            tele.jitter_target_ms.set(target as f32 * 1000.0 / SR as f32);
            inter.clear();
            for &s in &mono[..n] {
                for _ in 0..ch {
                    inter.push(s);
                }
            }
            stream.write(&inter)?;
            // Latency probe: find burst onsets in what we just wrote and ask the device
            // clock when they will be heard.
            {
                let mut probe = shared.probe.lock();
                if let Some(pr) = probe.as_mut() {
                    for (i, &x) in mono[..n].iter().enumerate() {
                        if let Some(()) = pr.feed(x) {
                            if let Some(t) = stream.play_time_100ns(frames_written + i as u64) {
                                pr.onsets.push(t);
                            }
                        }
                    }
                }
            }
            frames_written += n as u64;
            let q = stream.queued_frames().unwrap_or(0);
            tele.device_queue_ms.set(q as f32 * 1000.0 / SR as f32);
            tele.output_queue_ms.set(cons.occupied_len() as f32 * 1000.0 / SR as f32);
            tele.drift_ppm.set(((drift.ratio() - 1.0) * 1e6) as f32);
        })();
        if let Err(e) = res {
            tele.output_ok.store(false, Ordering::Relaxed);
            shared.emit(Level::Warn, format!("Output interrupted: {e:#}. Reconnecting…"));
            backoff_sleep(&shared, backoff);
        }
    }
}
