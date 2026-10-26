//! True end-to-end latency measurement.
//!
//! A helper copy of Voiceolator (`--beacon`) plays short tone bursts on the default
//! output device, exactly like a browser would. For each burst it asks its own device
//! clock *when the burst is actually played* (IAudioClock position + performance counter).
//! The main process captures the helper with the same process-loopback path used for
//! the stream, runs it through the full pipeline (model in bypass: the bypass path is
//! delay-matched to the model, so latency is identical) and asks *its* output clock when
//! each burst onset is played on your headphones. Both timestamps come from the system
//! performance counter, so their difference is the real delay you hear relative to the
//! original, minus only the DAC/headphone delay (Bluetooth adds its own on top).
//!
//! The helper's session is turned down in the mixer (and compensated in capture) so the
//! bursts are only heard once, through the measurement path.

use super::{discover, streams};
use crate::backend::{CaptureSource, OutputTarget};
use crate::host::{AudioHost, HostConfig, Probe};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use vo_dsp::{EngineKind, Settings};
use wasapi::{initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

const SR: usize = 48000;
const BURST_PERIOD: usize = SR * 7 / 10;
const BURST_LEN: usize = SR / 25; // 40 ms

fn burst_sample(i: usize) -> f32 {
    // 40 ms, 1 kHz + 2.3 kHz tone with 5 ms raised-cosine edges, -12 dBFS
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

/// Helper-process entry point: play bursts for `seconds`, printing `B <qpc100ns>` for
/// the play time of each detected onset.
pub fn run_beacon(seconds: f64) -> Result<()> {
    let _ = initialize_mta();
    let dev = DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
    let mut client = dev.get_iaudioclient()?;
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, SR, 2, None);
    client.initialize_client(&fmt, &Direction::Render, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 200_000 })?;
    let ev = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let clock = client.get_audioclock()?;
    let freq = clock.get_frequency()? as f64;
    let total = (seconds * SR as f64) as usize;
    let lead_in = SR; // 1 s silence so the parent can mute our session first
    let mut idx = 0usize;
    let mut detector = Probe::default();
    let mut pending: Vec<usize> = Vec::new();
    let mut started = false;
    let mut out = std::io::stdout();
    let mut buf: Vec<f32> = Vec::new();
    while idx < total {
        if started {
            ev.wait_for_event(500)?;
        }
        let n = client.get_available_space_in_frames()? as usize;
        buf.clear();
        for k in 0..n {
            let i = idx + k;
            let s = if i >= lead_in && (i - lead_in) % BURST_PERIOD < BURST_LEN { burst_sample((i - lead_in) % BURST_PERIOD) } else { 0.0 };
            if detector.feed(s).is_some() {
                pending.push(i);
            }
            buf.push(s);
            buf.push(s);
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len() * 4) };
        render.write_to_device(n, bytes, None)?;
        if !started {
            client.start_stream()?;
            started = true;
        }
        idx += n;
        if !pending.is_empty() {
            let (pos, qpc) = clock.get_position()?;
            let played_s = pos as f64 / freq;
            for i in pending.drain(..) {
                let t = qpc as i64 + ((i as f64 / SR as f64 - played_s) * 1e7) as i64;
                writeln!(out, "B {t}")?;
            }
            out.flush()?;
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    let _ = client.stop_stream();
    Ok(())
}

#[derive(Clone, Debug)]
pub struct LatencyReport {
    /// One measurement per burst (ms).
    pub samples_ms: Vec<f64>,
    pub median_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub engine: String,
}

/// Measure the delay from "the source app plays a sound" to "Voiceolator plays it on
/// `output`". `exe` is the path of a Voiceolator executable that understands `--beacon`.
pub fn measure(exe: &std::path::Path, engine: EngineKind, output: OutputTarget, buffer_ms: f32, seconds: f64) -> Result<LatencyReport> {
    let mut child = Command::new(exe)
        .arg("--beacon")
        .arg(format!("{seconds}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the latency beacon")?;
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

    let src = CaptureSource::Process { pid, name: discover::process_name(pid).unwrap_or_else(|| "beacon".into()) };
    // Hide the helper's own playback (turned down in the mixer, compensated in capture),
    // exactly like the "hide original sound" option does for the browser.
    let (input, duck) = super::duck::ducked_input_factory(src, super::duck::DuckTarget::Pid(pid));
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(900) {
        if duck.apply_now().unwrap_or(0) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let mut settings = Settings::default();
    settings.enabled = false; // delay-matched bypass: same latency, keeps the bursts intact
    settings.volume = 1.0;
    let label = engine.label().to_string();
    let host = AudioHost::start(HostConfig {
        input,
        output: streams::output_factory(output, buffer_ms),
        engine,
        settings,
        output_buffer_ms: buffer_ms,
        record: None,
    })?;
    host.start_probe();
    let deadline = Instant::now() + Duration::from_secs_f64(seconds + 1.0);
    let mut exited = false;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            exited = true;
            std::thread::sleep(Duration::from_millis(500)); // let the last burst through
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let onsets = host.take_probe();
    host.stop();
    duck.restore();
    if !exited {
        let _ = child.kill();
    }
    let _ = child.wait();
    let beacons: Vec<i64> = rx.try_iter().collect();
    if beacons.is_empty() {
        bail!("the test-tone helper produced no output (is an output device available?)");
    }
    let mut samples = Vec::new();
    for &o in &onsets {
        // Pair with the latest burst played before this onset, within 1 s.
        if let Some(&b) = beacons.iter().filter(|&&b| b <= o && o - b < 10_000_000).max() {
            samples.push((o - b) as f64 / 10_000.0);
        }
    }
    if samples.len() < 3 {
        bail!(
            "only {} of {} test bursts came through ({} onsets detected). Check that the output device works and that nothing else is muting the path.",
            samples.len(), beacons.len(), onsets.len()
        );
    }
    let mut s = samples.clone();
    s.sort_by(|a, b| a.total_cmp(b));
    Ok(LatencyReport { median_ms: s[s.len() / 2], min_ms: s[0], max_ms: s[s.len() - 1], samples_ms: samples, engine: label })
}
