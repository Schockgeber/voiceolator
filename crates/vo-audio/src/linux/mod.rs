//! Linux backend (PulseAudio or PipeWire via pipewire-pulse).
//!
//! **Per-app capture without any user setup.** On start Voiceolator creates a private,
//! silent virtual output ("Voiceolator-capture", a null sink) and moves only the chosen
//! app's audio streams into it. It records that sink's monitor, isolates the voice and
//! plays the result to your normal output. The original stream therefore no longer
//! reaches your speakers (nothing to mute) and nothing else is captured. On stop, every
//! moved stream goes back to the output it came from and the virtual sink is removed.
//! If Voiceolator crashes, [`recover`] (run at the next start) removes the leftover sink,
//! and the sound server moves the streams back to the default output by itself.

pub mod pa;

use crate::backend::{InputFactory, InputStream, OutputFactory, OutputStream, OutputTarget};
use crate::host::{AudioHost, HostConfig, Probe};
use anyhow::{bail, Context, Result};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use pa::Conn;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use vo_dsp::{EngineKind, Settings};

const SR: u32 = 48000;
const CH: u8 = 2;
pub const CAPTURE_SINK: &str = "voiceolator_capture";
const PROBE_SINK: &str = "voiceolator_probe";

fn spec() -> Spec {
    Spec { format: Format::F32le, channels: CH, rate: SR }
}

fn bytes_per_frame() -> usize {
    4 * CH as usize
}

/// Monotonic clock in 100 ns units (same clock in every process).
pub fn monotonic_100ns() -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as i64 * 10_000_000 + ts.tv_nsec as i64 / 100
}

pub(crate) fn normalise(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = s.rsplit('/').next().unwrap_or(&s).to_string();
    s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
}

// ------------------------------------------------------------------ discovery

#[derive(Clone, Debug)]
pub struct AudioApp {
    /// Identifier used for selection (executable name, e.g. "firefox").
    pub exe: String,
    pub pid: u32,
    pub streams: usize,
    pub playing: bool,
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

fn is_ours(i: &pa::InputRec, modules: &[u32]) -> bool {
    i.pid == std::process::id() || i.owner_module.map(|m| modules.contains(&m)).unwrap_or(false) || i.app_key().starts_with("voiceolator")
}

/// Applications that currently have audio streams.
pub fn list_audio_apps() -> Result<Vec<AudioApp>> {
    let mut c = Conn::open()?;
    let our_modules: Vec<u32> = c.sinks()?.iter().filter(|s| s.name.starts_with("voiceolator")).filter_map(|s| s.owner_module).collect();
    let mut by: HashMap<String, AudioApp> = HashMap::new();
    for i in c.sink_inputs()? {
        if is_ours(&i, &our_modules) || i.client.is_none() {
            continue; // our own streams, loopbacks and other server-internal streams
        }
        let key = normalise(&i.app_key());
        if key.is_empty() {
            continue;
        }
        let e = by.entry(key.clone()).or_insert(AudioApp { exe: key, pid: i.pid, streams: 0, playing: false });
        e.streams += 1;
        e.playing |= !i.corked;
    }
    let mut v: Vec<AudioApp> = by.into_values().collect();
    v.sort_by(|a, b| b.playing.cmp(&a.playing).then(a.exe.cmp(&b.exe)));
    Ok(v)
}

pub fn find_app(name: &str) -> Result<Option<AudioApp>> {
    let want = normalise(name);
    Ok(list_audio_apps()?.into_iter().find(|a| a.exe == want))
}

pub fn list_output_devices() -> Result<Vec<DeviceInfo>> {
    let mut c = Conn::open()?;
    let def = c.server_default_sink()?.unwrap_or_default();
    Ok(c
        .sinks()?
        .into_iter()
        .filter(|s| !s.name.starts_with("voiceolator"))
        .map(|s| DeviceInfo { is_default: s.name == def, name: if s.description.is_empty() { s.name.clone() } else { s.description }, id: s.name })
        .collect())
}

pub fn find_output(query: &str) -> Result<Option<DeviceInfo>> {
    let q = query.to_lowercase();
    let devs = list_output_devices()?;
    Ok(devs.iter().find(|d| d.id == query || d.name == query).or_else(|| devs.iter().find(|d| d.name.to_lowercase().contains(&q))).cloned())
}

/// Remove virtual sinks left behind by a run that did not exit cleanly. Their streams
/// are moved back to the default output by the sound server.
pub fn recover() -> Result<usize> {
    let mut c = Conn::open()?;
    let mut n = 0;
    for s in c.sinks()? {
        if s.name.starts_with("voiceolator") {
            if let Some(m) = s.owner_module {
                if c.unload_module(m)? {
                    n += 1;
                }
            }
        }
    }
    Ok(n)
}

// ------------------------------------------------------------------ routing

#[derive(Clone, Debug, PartialEq)]
pub enum RouteTarget {
    App(String),
    Pid(u32),
}

/// Owns the private capture sink and the streams moved into it.
pub struct Router {
    target: RouteTarget,
    sink: String,
    sink_module: Option<u32>,
    loopback_module: Option<u32>,
    keep_original_audible: bool,
    original: HashMap<u32, String>,
}

impl Router {
    pub fn new(target: RouteTarget, sink: &str, keep_original_audible: bool) -> Self {
        Self { target, sink: sink.to_string(), sink_module: None, loopback_module: None, keep_original_audible, original: HashMap::new() }
    }

    pub fn monitor(&self) -> String {
        format!("{}.monitor", self.sink)
    }

    fn ensure_sink(&mut self, c: &mut Conn) -> Result<()> {
        let sinks = c.sinks()?;
        if let Some(s) = sinks.iter().find(|s| s.name == self.sink) {
            if self.sink_module.is_none() {
                self.sink_module = s.owner_module; // leftover from a crash: adopt it
            }
        } else {
            let args = format!("sink_name={} rate={SR} channels={CH} format=float32le sink_properties=device.description=Voiceolator-capture", self.sink);
            self.sink_module = Some(c.load_module("module-null-sink", &args).context("creating the Voiceolator capture sink")?);
            crate::watchdog::spawn_once();
        }
        if self.keep_original_audible && self.loopback_module.is_none() {
            if let Some(def) = c.server_default_sink()? {
                let args = format!("source={} sink={def} latency_msec=40", self.monitor());
                self.loopback_module = c.load_module("module-loopback", &args).ok();
            }
        }
        Ok(())
    }

    /// Move the target's streams into the capture sink. Returns how many were moved.
    pub fn apply(&mut self) -> Result<usize> {
        let mut c = Conn::open()?;
        self.ensure_sink(&mut c)?;
        let sinks = c.sinks()?;
        let Some(ours) = sinks.iter().find(|s| s.name == self.sink).map(|s| s.index) else { bail!("capture sink disappeared") };
        let me = std::process::id();
        let mut moved = 0;
        for i in c.sink_inputs()? {
            if i.sink == ours || i.pid == me || i.client.is_none() {
                continue;
            }
            let hit = match &self.target {
                RouteTarget::App(a) => normalise(&i.app_key()) == normalise(a),
                RouteTarget::Pid(p) => i.pid == *p,
            };
            if !hit {
                continue;
            }
            let from = sinks.iter().find(|s| s.index == i.sink).map(|s| s.name.clone()).unwrap_or_default();
            if c.move_input(i.index, &self.sink)? {
                self.original.insert(i.index, from);
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Put every moved stream back and remove the capture sink.
    pub fn restore(&mut self) {
        let Ok(mut c) = Conn::open() else { return };
        let ours = c.sinks().ok().and_then(|s| s.iter().find(|s| s.name == self.sink).map(|s| s.index));
        if let (Ok(inputs), Some(ours)) = (c.sink_inputs(), ours) {
            for i in inputs.iter().filter(|i| i.sink == ours) {
                if let Some(orig) = self.original.get(&i.index) {
                    if !orig.is_empty() {
                        let _ = c.move_input(i.index, orig);
                    }
                }
            }
        }
        self.original.clear();
        if let Some(m) = self.loopback_module.take() {
            let _ = c.unload_module(m);
        }
        if let Some(m) = self.sink_module.take() {
            let _ = c.unload_module(m); // remaining streams fall back to the default output
        }
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.restore();
    }
}

pub struct RouteShared {
    router: Mutex<Router>,
}

impl RouteShared {
    pub fn restore(&self) {
        self.router.lock().restore();
    }
    pub fn apply_now(&self) -> Result<usize> {
        self.router.lock().apply()
    }
}

// ------------------------------------------------------------------ streams

pub struct PaInput {
    s: Simple,
    buf: Vec<u8>,
    shared: Option<Arc<RouteShared>>,
    last_apply: Instant,
}

impl PaInput {
    pub fn open_monitor(monitor: &str, shared: Option<Arc<RouteShared>>) -> Result<Self> {
        let frag = (SR as usize / 100) * bytes_per_frame(); // 10 ms
        let attr = BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: frag as u32 };
        let s = Simple::new(None, "Voiceolator", Direction::Record, Some(monitor), "capture", &spec(), None, Some(&attr))
            .map_err(|e| anyhow::anyhow!("cannot record from {monitor}: {e}"))?;
        Ok(Self { s, buf: vec![0u8; frag], shared, last_apply: Instant::now() })
    }
}

impl InputStream for PaInput {
    fn sample_rate(&self) -> u32 {
        SR
    }
    fn channels(&self) -> usize {
        CH as usize
    }
    fn latency_frames(&self) -> usize {
        SR as usize / 100
    }
    fn read(&mut self, out: &mut Vec<f32>, _timeout_ms: u32) -> Result<usize> {
        self.s.read(&mut self.buf).map_err(|e| anyhow::anyhow!("capture stream: {e}"))?;
        out.extend(self.buf.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        if let Some(sh) = &self.shared {
            // New streams (e.g. another tab starts playing) are picked up within 0.5 s.
            if self.last_apply.elapsed() > Duration::from_millis(500) {
                self.last_apply = Instant::now();
                if let Err(e) = sh.apply_now() {
                    log::warn!("routing update failed: {e:#}");
                }
            }
        }
        Ok(self.buf.len() / bytes_per_frame())
    }
}

pub struct PaOutput {
    s: Simple,
    written: u64,
    hop: usize,
}

impl PaOutput {
    pub fn open(target: &OutputTarget, buffer_ms: f32) -> Result<Self> {
        let bpf = bytes_per_frame() as u32;
        let tlength = ((buffer_ms.max(10.0) / 1000.0) * SR as f32) as u32 * bpf;
        let attr = BufferAttr { maxlength: u32::MAX, tlength, prebuf: u32::MAX, minreq: (SR / 100) * bpf, fragsize: u32::MAX };
        let s = Simple::new(None, "Voiceolator", Direction::Playback, target.id.as_deref(), "Isolated voice", &spec(), None, Some(&attr))
            .map_err(|e| anyhow::anyhow!("cannot play to {}: {e}", target.name))?;
        Ok(Self { s, written: 0, hop: SR as usize / 100 })
    }
}

impl OutputStream for PaOutput {
    fn sample_rate(&self) -> u32 {
        SR
    }
    fn channels(&self) -> usize {
        CH as usize
    }
    fn wait(&mut self, _timeout_ms: u32) -> Result<()> {
        Ok(()) // write() blocks until the server wants more
    }
    fn writable_frames(&mut self) -> Result<usize> {
        Ok(self.hop)
    }
    fn queued_frames(&mut self) -> Result<usize> {
        let us = self.s.get_latency().map(|l| l.0).unwrap_or(0);
        Ok((us as usize * SR as usize) / 1_000_000)
    }
    fn write(&mut self, data: &[f32]) -> Result<()> {
        // SAFETY: f32 has no invalid bit patterns, u8 has alignment 1
        let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) };
        self.s.write(bytes).map_err(|e| anyhow::anyhow!("output stream: {e}"))?;
        self.written += (data.len() / CH as usize) as u64;
        Ok(())
    }
    fn play_time_100ns(&mut self, frame_index: u64) -> Option<i64> {
        let lat_us = self.s.get_latency().ok()?.0 as i64;
        let now = monotonic_100ns();
        let behind = self.written.saturating_sub(1).saturating_sub(frame_index) as i64; // frames queued after it
        Some(now + lat_us * 10 - behind * 10_000_000 / SR as i64)
    }
}

/// Capture an app (or process) by routing its streams into a private sink.
pub fn routed_input_factory(target: RouteTarget, sink: &str, keep_original_audible: bool) -> (InputFactory, Arc<RouteShared>) {
    let shared = Arc::new(RouteShared { router: Mutex::new(Router::new(target, sink, keep_original_audible)) });
    let s2 = shared.clone();
    let f: InputFactory = Box::new(move || {
        s2.apply_now()?; // creates the sink and moves the app's streams
        let monitor = s2.router.lock().monitor();
        Ok(Box::new(PaInput::open_monitor(&monitor, Some(s2.clone()))?) as Box<dyn InputStream>)
    });
    (f, shared)
}

pub fn output_factory(target: OutputTarget, buffer_ms: f32) -> OutputFactory {
    Box::new(move || Ok(Box::new(PaOutput::open(&target, buffer_ms)?) as Box<dyn OutputStream>))
}

// ------------------------------------------------------------------ latency

const BURST_PERIOD: usize = SR as usize * 7 / 10;
const BURST_LEN: usize = SR as usize / 25;

fn burst_sample(i: usize) -> f32 {
    let t = i as f32 / SR as f32;
    let edge = (SR / 200) as f32;
    let fi = i as f32;
    let env = if fi < edge {
        0.5 - 0.5 * (std::f32::consts::PI * fi / edge).cos()
    } else if fi > BURST_LEN as f32 - edge {
        0.5 - 0.5 * (std::f32::consts::PI * (BURST_LEN as f32 - fi) / edge).cos()
    } else {
        1.0
    };
    0.25 * env * (0.7 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin() + 0.3 * (2.0 * std::f32::consts::PI * 2300.0 * t).sin())
}

/// Helper-process entry point: play test bursts to the default output, printing
/// `B <monotonic 100ns>` for the play time of each burst onset.
pub fn run_beacon(seconds: f64) -> Result<()> {
    use std::io::Write;
    let bpf = bytes_per_frame() as u32;
    let attr = BufferAttr { maxlength: u32::MAX, tlength: (SR / 50) * bpf, prebuf: u32::MAX, minreq: (SR / 100) * bpf, fragsize: u32::MAX };
    let s = Simple::new(None, "Voiceolator latency test", Direction::Playback, None, "test tone", &spec(), None, Some(&attr))
        .map_err(|e| anyhow::anyhow!("beacon: {e}"))?;
    let total = (seconds * SR as f64) as usize;
    let lead_in = SR as usize;
    let block = SR as usize / 100;
    let mut det = Probe::default();
    let mut out = std::io::stdout();
    let mut idx = 0usize;
    let mut buf = Vec::with_capacity(block * 2);
    while idx < total {
        buf.clear();
        let mut onset = None;
        for k in 0..block {
            let i = idx + k;
            let v = if i >= lead_in && (i - lead_in) % BURST_PERIOD < BURST_LEN { burst_sample((i - lead_in) % BURST_PERIOD) } else { 0.0 };
            if det.feed(v).is_some() {
                onset = Some(i);
            }
            buf.push(v);
            buf.push(v);
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 4) };
        s.write(bytes).map_err(|e| anyhow::anyhow!("beacon write: {e}"))?;
        idx += block;
        if let Some(i) = onset {
            let lat = s.get_latency().map(|l| l.0 as i64).unwrap_or(0);
            let behind = (idx - 1 - i) as i64;
            let t = monotonic_100ns() + lat * 10 - behind * 10_000_000 / SR as i64;
            writeln!(out, "B {t}")?;
            out.flush()?;
        }
    }
    let _ = s.drain();
    Ok(())
}

#[derive(Clone, Debug)]
pub struct LatencyReport {
    pub samples_ms: Vec<f64>,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub engine: String,
}

/// Measure the delay from "an app plays a sound" to "Voiceolator plays it on `output`".
pub fn measure_latency(exe: &std::path::Path, engine: EngineKind, output: OutputTarget, buffer_ms: f32, seconds: f64) -> Result<LatencyReport> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut child = Command::new(exe).arg("--beacon").arg(format!("{seconds}")).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().context("starting the latency beacon")?;
    let pid = child.id();
    let stdout = child.stdout.take().context("beacon stdout")?;
    let (tx, rx) = std::sync::mpsc::channel::<i64>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(v) = line.strip_prefix("B ").and_then(|v| v.trim().parse().ok()) {
                let _ = tx.send(v);
            }
        }
    });
    let (input, route) = routed_input_factory(RouteTarget::Pid(pid), PROBE_SINK, false);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(900) {
        if route.apply_now().unwrap_or(0) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let mut settings = Settings::default();
    settings.enabled = false;
    settings.volume = 1.0;
    let label = engine.label().to_string();
    let host = AudioHost::start(HostConfig { input, output: output_factory(output, buffer_ms), engine, settings, output_buffer_ms: buffer_ms, record: None })?;
    host.start_probe();
    let deadline = Instant::now() + Duration::from_secs_f64(seconds + 1.0);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            std::thread::sleep(Duration::from_millis(500));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let onsets = host.take_probe();
    host.stop();
    route.restore();
    let _ = child.kill();
    let _ = child.wait();
    let beacons: Vec<i64> = rx.try_iter().collect();
    if beacons.is_empty() {
        bail!("the test-tone helper produced no output");
    }
    let mut samples = Vec::new();
    for &o in &onsets {
        if let Some(&b) = beacons.iter().filter(|&&b| b <= o && o - b < 10_000_000).max() {
            samples.push((o - b) as f64 / 10_000.0);
        }
    }
    if samples.len() < 3 {
        bail!("only {} of {} test bursts came through ({} onsets detected)", samples.len(), beacons.len(), onsets.len());
    }
    let mut s = samples.clone();
    s.sort_by(|a, b| a.total_cmp(b));
    Ok(LatencyReport { median_ms: s[s.len() / 2], min_ms: s[0], max_ms: s[s.len() - 1], samples_ms: samples, engine: label })
}
