//! End-to-end host test with simulated real-time devices (no audio hardware needed):
//! verifies continuous output, latency, clock-drift absorption and recovery from a
//! simulated device disconnect.

use parking_lot::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use vo_audio::testing::{SimInput, SimOutput};
use vo_audio::{AudioHost, HostConfig};
use vo_dsp::{EngineKind, Settings};

fn tone_bursts(secs: usize) -> Vec<f32> {
    // 200 ms "voice-like" harmonic bursts every 500 ms
    (0..48000 * secs)
        .map(|i| {
            let t = i as f32 / 48000.0;
            let on = (t % 0.5) < 0.2;
            if on {
                0.1 * ((2.0 * std::f32::consts::PI * 180.0 * t).sin() + 0.5 * (2.0 * std::f32::consts::PI * 360.0 * t).sin())
            } else {
                0.0
            }
        })
        .collect()
}

/// These tests measure real-time behaviour, so they must not compete for the CPU.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(secs: usize, in_scale: f64, out_scale: f64, fail_at: Option<usize>) -> (Vec<f32>, Arc<vo_audio::Telemetry>, Vec<String>) {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let sig = Arc::new(tone_bursts(secs + 1));
    let pos = Arc::new(AtomicUsize::new(0));
    let sink = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(fail_at));
    let (s2, p2, f2) = (sig.clone(), pos.clone(), fail.clone());
    let sink2 = sink.clone();
    let mut settings = Settings::default();
    settings.leveler = false;
    let host = AudioHost::start(HostConfig {
        input: Box::new(move || Ok(Box::new(SimInput::new(s2.clone(), p2.clone(), in_scale, f2.lock().take())))),
        output: Box::new(move || Ok(Box::new(SimOutput::new(sink2.clone(), out_scale)))),
        engine: EngineKind::Dfn3,
        settings,
        output_buffer_ms: 20.0,
        record: None,
    })
    .unwrap();
    std::thread::sleep(Duration::from_secs(secs as u64));
    let tele = host.telemetry();
    let events = host.poll_events().into_iter().map(|e| e.message).collect();
    host.stop();
    let out = sink.lock().clone();
    (out, tele, events)
}

#[test]
fn streams_continuously_with_bounded_latency() {
    let (out, tele, events) = run(6, 1.0, 1.0, None);
    println!("events: {events:?}");
    let total = tele.total_latency_ms();
    println!(
        "latency est {total:.1} ms (cap {:.1} inq {:.1} eng {:.1} proc {:.2} outq {:.1} dev {:.1}), load {:.3}, underruns {}",
        tele.capture_latency_ms.get(), tele.input_queue_ms.get(), tele.engine_latency_ms.get(), tele.proc_ms_avg.get(),
        tele.output_queue_ms.get(), tele.device_queue_ms.get(), tele.dsp_load.get(),
        tele.underruns.load(std::sync::atomic::Ordering::Relaxed)
    );
    assert!(out.len() > 48000 * 5, "too little output: {}", out.len());
    // generous bound: this runs on loaded CI machines; typical value is 80-90 ms
    assert!(total < 130.0, "latency estimate {total} ms");
    // output must contain the bursts (not silence)
    let e: f32 = out[48000..].iter().map(|x| x * x).sum::<f32>() / (out.len() - 48000) as f32;
    assert!(e > 1e-5, "output is silent (energy {e})");
    // measure actual delay vs input by cross-correlating burst envelopes
    let sig = tone_bursts(7);
    let env = |x: &[f32]| x.chunks(48).map(|c| c.iter().map(|v| v * v).sum::<f32>()).collect::<Vec<f32>>();
    let (ei, eo) = (env(&sig[..48000 * 5]), env(&out[..48000 * 5]));
    let best = (0..150usize)
        .max_by(|&a, &b| {
            let ca: f32 = (0..ei.len() - 150).map(|i| ei[i] * eo[i + a]).sum();
            let cb: f32 = (0..ei.len() - 150).map(|i| ei[i] * eo[i + b]).sum();
            ca.partial_cmp(&cb).unwrap()
        })
        .unwrap();
    println!("measured stream delay (sim clock, excl. device): {} ms", best);
}

#[test]
fn absorbs_clock_drift() {
    // capture clock 0.2 % fast vs output clock: without compensation the queue would grow
    // by 2 ms per second.
    let (_out, tele, _) = run(8, 1.002, 1.0, None);
    let q = tele.output_queue_ms.get();
    println!("output queue after 8 s with 0.2 % drift: {q:.1} ms, ratio ppm {:.0}", tele.drift_ppm.get());
    assert!(q < 45.0, "queue grew to {q} ms");
}

#[test]
fn recovers_from_device_disconnect() {
    let (out, tele, events) = run(6, 1.0, 1.0, Some(48000 * 2));
    println!("events: {events:?}");
    assert!(events.iter().any(|e| e.contains("simulated device removal")));
    assert!(tele.restarts.load(std::sync::atomic::Ordering::Relaxed) >= 1);
    // audio resumes after the disconnect
    let tail = &out[out.len().saturating_sub(48000)..];
    let e: f32 = tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32;
    assert!(e > 1e-5, "no audio after reconnect");
}
