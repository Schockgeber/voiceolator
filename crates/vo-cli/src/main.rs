//! voiceolator-cli — Stage 3 prototype.
//!
//! ```text
//! voiceolator-cli apps                       list applications currently playing audio
//! voiceolator-cli devices                    list output and input devices
//! voiceolator-cli run --app chrome [options] capture an app, isolate voice, play it
//! voiceolator-cli run --input "CABLE Output" capture an input device (e.g. VB-CABLE)
//!
//! run options:
//!   --output NAME        output device (substring of the name; default = system default)
//!   --engine dpdfnet|dfn3   voice model (default dpdfnet, falls back to dfn3)
//!   --preset balanced|voice|max
//!   --buffer MS          output jitter buffer (default 20)
//!   --record DIR         save original.wav and processed.wav for A/B comparison
//!   --seconds N          stop after N seconds
//! While running, type a command + Enter:  b = toggle voice isolation (A/B),
//!   1/2/3 = Balanced / Voice Priority / Maximum Suppression,  + / - = volume,  q = quit
//! ```

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vo_dsp::{EngineKind, Preset, Settings};

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| ".".into())
}

/// Locate model + ONNX Runtime next to the executable (installer layout).
fn dpdfnet_kind() -> Option<EngineKind> {
    let d = exe_dir();
    let model = d.join("models").join("dpdfnet2_48khz_hr.onnx");
    let ort = d.join(if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so" });
    (model.exists() && ort.exists()).then_some(EngineKind::Dpdfnet { model, onnxruntime: ort })
}

struct Args {
    cmd: String,
    app: Option<String>,
    input: Option<String>,
    output: Option<String>,
    engine: String,
    preset: Preset,
    buffer_ms: f32,
    record: Option<PathBuf>,
    seconds: Option<f64>,
    mute_original: bool,
}

fn parse() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_else(|| "help".into());
    let mut a = Args { cmd, app: None, input: None, output: None, engine: "dpdfnet".into(), preset: Preset::Balanced, buffer_ms: 20.0, record: None, seconds: None, mute_original: true };
    while let Some(k) = it.next() {
        let mut v = || it.next().with_context(|| format!("{k} needs a value"));
        match k.as_str() {
            "--app" => a.app = Some(v()?),
            "--input" => a.input = Some(v()?),
            "--output" => a.output = Some(v()?),
            "--engine" => a.engine = v()?,
            "--preset" => a.preset = Preset::parse(&v()?).context("preset must be balanced|voice|max")?,
            "--buffer" => a.buffer_ms = v()?.parse()?,
            "--record" => a.record = Some(v()?.into()),
            "--seconds" => a.seconds = Some(v()?.parse()?),
            "--mute-original" | "--hide-original" => a.mute_original = true,
            "--keep-original" => a.mute_original = false,
            _ => bail!("unknown option {k}"),
        }
    }
    Ok(a)
}

const HELP: &str = "voiceolator-cli: real-time streamer voice isolation (prototype)

  voiceolator-cli apps
  voiceolator-cli devices
  voiceolator-cli run --app chrome [--output \"Headphones\"] [--engine dpdfnet|dfn3]
                      [--preset balanced|voice|max] [--record DIR] [--seconds N]
  voiceolator-cli run --input \"CABLE Output\" ...

  voiceolator-cli run ... --keep-original      also let the original app sound play
                                              (default: it is turned down while running)
  voiceolator-cli latency [--output NAME] [--engine dpdfnet|dfn3]
                                              measure the real end-to-end delay (plays beeps)
  voiceolator-cli bench                       measure model CPU cost on this PC
  voiceolator-cli process FILE... [--out DIR] [--format wav|mp3|m4a|flac] [--audio-only]
                  [--keep-original-track] [--preset balanced|voice|max] [--engine quality|light] [--threads N]
                                              isolate the voice in audio or video files
                                              (writes \"NAME (voice).ext\" next to each file)
  voiceolator-cli ffmpeg [--download [--yes] | --remove]
                                              show / download / remove ffmpeg (for video + MP3/M4A/FLAC)

While running type + Enter: b = toggle isolation (A/B), 1/2/3 = presets, +/- = volume, q = quit";

fn main() -> Result<()> {
    vo_audio::watchdog::handle_args();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let first = std::env::args().nth(1).unwrap_or_default();
    // Commands with positional arguments are handled before option parsing.
    match first.as_str() {
        "--beacon" => return platform::beacon(),
        "process" => return process_file(),
        "ffmpeg" => return ffmpeg_cmd(),
        _ => {}
    }
    let a = parse()?;
    match a.cmd.as_str() {
        "apps" => platform::apps(),
        "devices" => platform::devices(),
        "run" => run(a),
        "bench" => bench(),
        "latency" => platform::latency(&a),
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

/// Run each available engine on 20 s of synthetic audio and report the real-time cost.
fn bench() -> Result<()> {
    let n = 48000 * 20;
    let mut seed = 1u32;
    let x: Vec<f32> = (0..n)
        .map(|i| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
            let t = i as f32 / 48000.0;
            0.1 * (2.0 * std::f32::consts::PI * 150.0 * t).sin() * ((t * 3.0).sin() > 0.0) as i32 as f32 + 0.05 * noise
        })
        .collect();
    let mut kinds = vec![EngineKind::Dfn3];
    if let Some(k) = dpdfnet_kind() {
        kinds.insert(0, k);
    } else {
        println!("(DPDFNet files not found next to the exe; benchmarking DeepFilterNet3 only)");
    }
    for kind in kinds {
        let settings = Settings::default();
        let t_load = Instant::now();
        let engine = match vo_dsp::create_engine(&kind, &settings) {
            Ok(e) => e,
            Err(e) => {
                println!("{:<24} failed to load: {e:#}", kind.label());
                continue;
            }
        };
        let load = t_load.elapsed();
        let mut p = vo_dsp::Pipeline::new(engine, settings);
        let hop = p.hop_size();
        let mut out = vec![0.0; hop];
        let mut times = Vec::with_capacity(n / hop);
        for c in x.chunks_exact(hop) {
            let t = Instant::now();
            p.process_hop(c, &mut out)?;
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.total_cmp(b));
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let p99 = times[times.len() * 99 / 100];
        println!(
            "{:<24} load {:>5.0} ms | per 10 ms hop: mean {:.2} ms, p99 {:.2} ms, max {:.2} ms | = {:.1}% of one CPU core | algorithmic latency {:.0} ms",
            p.engine_name(), load.as_secs_f64() * 1000.0, mean, p99, times[times.len() - 1], mean / 10.0 * 100.0,
            (p.latency_samples() + hop) as f64 / 48.0
        );
    }
    Ok(())
}

fn process_file() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut out_dir: Option<PathBuf> = None;
    let mut format = vo_media::AudioFormat::Wav;
    let mut preset = Preset::Balanced;
    let mut light = false;
    let mut make_video = true;
    let mut keep_original = false;
    let mut use_ffmpeg = true;
    let mut threads = 0usize;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut v = || it.next().cloned().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--out" | "-o" => out_dir = Some(v()?.into()),
            "--format" => format = vo_media::AudioFormat::parse(&v()?).context("format must be wav|mp3|m4a|flac")?,
            "--preset" => preset = Preset::parse(&v()?).context("preset must be balanced|voice|max")?,
            "--engine" => light = matches!(v()?.as_str(), "dfn3" | "light"),
            "--audio-only" => make_video = false,
            "--keep-original-track" => keep_original = true,
            "--no-ffmpeg" => use_ffmpeg = false,
            "--threads" => threads = v()?.parse().context("--threads needs a number")?,
            // pre-0.11 positional form: process IN OUT [preset] [dfn3]
            "balanced" | "voice" | "max" => preset = Preset::parse(a).unwrap(),
            "dfn3" => light = true,
            f if f.starts_with("--") => bail!("unknown option {f}"),
            f => files.push(f.into()),
        }
    }
    // Old form "process in.wav out.wav": second path does not exist and is a .wav
    let mut single_out: Option<PathBuf> = None;
    if files.len() == 2 && !files[1].exists() && files[1].extension().map(|e| e.eq_ignore_ascii_case("wav")).unwrap_or(false) {
        single_out = files.pop();
    }
    if files.is_empty() {
        bail!("usage: voiceolator-cli process FILE... [--out DIR] [--format wav|mp3|m4a|flac] [--audio-only] [--keep-original-track] [--preset balanced|voice|max] [--engine quality|light] [--threads N] [--no-ffmpeg]");
    }
    let engine = if light { EngineKind::Dfn3 } else { dpdfnet_kind().unwrap_or(EngineKind::Dfn3) };
    let ffmpeg = if use_ffmpeg { vo_media::ffmpeg::find() } else { None };
    match &ffmpeg {
        Some(f) => println!("ffmpeg {} ({})", f.version, f.path.display()),
        None if use_ffmpeg => println!("ffmpeg not found: built-in decoder, WAV output only (run `voiceolator-cli ffmpeg --download` to get it)"),
        None => {}
    }
    let opts = vo_media::Options {
        engine,
        settings: Settings::from_preset(preset),
        out_dir,
        audio_format: format,
        make_video,
        keep_original_track: keep_original,
        ffmpeg: ffmpeg.map(|f| f.path),
        threads,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let c = cancel.clone();
        let _ = ctrlc::set_handler(move || c.store(true, Ordering::SeqCst));
    }
    let mut failed = 0;
    for f in &files {
        let mut last_pct = -1i32;
        let mut speed = 0.0f32;
        let r = vo_media::process(
            f,
            &opts,
            &mut |p| {
                if let Some(s) = p.speed {
                    speed = s;
                }
                if let Some(fr) = p.fraction {
                    let pct = (fr * 100.0) as i32;
                    if pct / 10 != last_pct / 10 {
                        last_pct = pct;
                        eprint!("\r{}: {:>3}%  ({:.1}x real time)   ", f.display(), pct, speed);
                    }
                }
            },
            &cancel,
        );
        eprint!("\r");
        match r {
            Ok(d) => {
                let out = match &single_out {
                    Some(o) => {
                        std::fs::rename(&d.output, o)?;
                        o.clone()
                    }
                    None => d.output.clone(),
                };
                println!("{} -> {}  ({:.0} s of audio in {:.1} s, {:.0}x real time, {})", f.display(), out.display(), d.audio_seconds, d.elapsed_s, d.audio_seconds / d.elapsed_s.max(1e-3), d.engine);
                for n in d.notes {
                    println!("  note: {n}");
                }
            }
            Err(e) if e.is::<vo_media::Cancelled>() => {
                println!("cancelled");
                std::process::exit(130);
            }
            Err(e) => {
                failed += 1;
                println!("{}: FAILED: {e:#}", f.display());
            }
        }
    }
    if failed > 0 {
        bail!("{failed} file(s) failed");
    }
    Ok(())
}

fn ffmpeg_cmd() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let download = args.iter().any(|a| a == "--download");
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    if args.iter().any(|a| a == "--remove") {
        vo_media::ffmpeg::remove_downloaded()?;
        println!("removed {}", vo_media::ffmpeg::managed_dir().display());
        return Ok(());
    }
    if let Some(f) = vo_media::ffmpeg::find() {
        println!("ffmpeg {} at {}", f.version, f.path.display());
        if !download {
            return Ok(());
        }
    } else if !download {
        println!("ffmpeg not found. `voiceolator-cli ffmpeg --download` downloads it (Voiceolator works without it for WAV files).");
        return Ok(());
    }
    let info = vo_media::ffmpeg::DOWNLOAD.as_ref().context("no ffmpeg download is available for this system; install ffmpeg with your package manager")?;
    println!(
        "Download ffmpeg {} ({} build, {} licence, {:.0} MB) from\n  {}\ninto {}?",
        info.version,
        info.publisher,
        info.license,
        info.size as f64 / 1e6,
        info.urls[0],
        vo_media::ffmpeg::managed_dir().display()
    );
    if !yes {
        print!("Type y + Enter to continue: ");
        std::io::Write::flush(&mut std::io::stdout())?;
        let mut l = String::new();
        std::io::stdin().read_line(&mut l)?;
        if !l.trim().eq_ignore_ascii_case("y") {
            println!("not downloaded");
            return Ok(());
        }
    }
    let cancel = AtomicBool::new(false);
    let f = vo_media::ffmpeg::download(
        &mut |s| match s {
            vo_media::ffmpeg::DownloadStage::Downloading { done, total } => eprint!("\rdownloading {:>3}%", done * 100 / total.max(1)),
            vo_media::ffmpeg::DownloadStage::Extracting => eprint!("\runpacking...      "),
        },
        &cancel,
    )?;
    eprintln!();
    println!("ffmpeg {} installed at {}", f.version, f.path.display());
    Ok(())
}

fn bar(db: f32) -> String {
    let n = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * 10.0).round() as usize;
    format!("{}{}", "#".repeat(n), ".".repeat(10 - n))
}

fn run(a: Args) -> Result<()> {
    let (input, output, source_desc, duck_guard) = platform::endpoints(&a)?;
    let engine = match a.engine.as_str() {
        "dfn3" => EngineKind::Dfn3,
        "dpdfnet" => dpdfnet_kind().unwrap_or_else(|| {
            eprintln!("note: models/dpdfnet2_48khz_hr.onnx or the ONNX Runtime library not found next to the exe; using DeepFilterNet3");
            EngineKind::Dfn3
        }),
        e => bail!("unknown engine {e}"),
    };
    let record = match &a.record {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            Some((dir.join("original.wav"), dir.join("processed.wav")))
        }
        None => None,
    };
    let host = vo_audio::AudioHost::start(vo_audio::HostConfig {
        input,
        output,
        engine,
        settings: Settings::from_preset(a.preset),
        output_buffer_ms: a.buffer_ms,
        record: record.clone(),
    })?;
    println!("Capturing {source_desc}. Type b+Enter to A/B, 1/2/3 presets, q to quit.");

    let quit = Arc::new(AtomicBool::new(false));
    let q2 = quit.clone();
    ctrlc::set_handler(move || q2.store(true, Ordering::Relaxed)).ok();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        while stdin.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
            let _ = tx.send(line.trim().to_string());
            line.clear();
        }
    });

    let t0 = Instant::now();
    let tele = host.telemetry();
    let mut last_print = Instant::now() - Duration::from_secs(1);
    while !quit.load(Ordering::Relaxed) {
        if let Some(s) = a.seconds {
            if t0.elapsed().as_secs_f64() >= s {
                break;
            }
        }
        for cmd in rx.try_iter() {
            let mut s = host.settings();
            match cmd.as_str() {
                "b" => s.enabled = !s.enabled,
                "1" => s = Settings { volume: s.volume, enabled: s.enabled, ..Settings::from_preset(Preset::Balanced) },
                "2" => s = Settings { volume: s.volume, enabled: s.enabled, ..Settings::from_preset(Preset::VoicePriority) },
                "3" => s = Settings { volume: s.volume, enabled: s.enabled, ..Settings::from_preset(Preset::MaximumSuppression) },
                "+" => s.volume = (s.volume * 1.26).min(4.0),
                "-" => s.volume /= 1.26,
                "m" => s.mute = !s.mute,
                "q" => quit.store(true, Ordering::Relaxed),
                _ => {}
            }
            host.set_settings(s);
            println!("\n> isolation {} | preset {} | volume {:+.1} dB{}", if s.enabled { "ON" } else { "OFF (original)" }, s.preset.label(), 20.0 * s.volume.max(1e-6).log10(), if s.mute { " | MUTED" } else { "" });
        }
        for e in host.poll_events() {
            println!("\n[{:?}] {}", e.level, e.message);
        }
        if last_print.elapsed() >= Duration::from_millis(500) {
            last_print = Instant::now();
            let s = host.settings();
            print!(
                "\r[{}] latency ~{:3.0} ms (model {:.0}) | DSP {:4.1}% core (max {:4.1} ms) | in {} {:5.1} dB | out {} {:5.1} dB | speech {:3.0}% | drop {} ",
                if s.enabled { "VOICE" } else { "ORIG " },
                tele.total_latency_ms(),
                tele.engine_latency_ms.get(),
                tele.dsp_load.get() * 100.0,
                tele.proc_ms_max.get(),
                bar(tele.input_peak_db.get()),
                tele.input_peak_db.get().max(-99.0),
                bar(tele.output_peak_db.get()),
                tele.output_peak_db.get().max(-99.0),
                tele.speech_activity.get() * 100.0,
                tele.underruns.load(Ordering::Relaxed),
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("\nStopping…");
    let summary = format!(
        "latency estimate {:.0} ms = capture {:.0} + input queue {:.0} + model {:.0} + compute {:.1} + output queue {:.0} + device buffer {:.0}; DSP load {:.1}% of one core; underruns {}, overruns {}, restarts {}",
        tele.total_latency_ms(), tele.capture_latency_ms.get(), tele.input_queue_ms.get(), tele.engine_latency_ms.get(), tele.proc_ms_avg.get(),
        tele.output_queue_ms.get(), tele.device_queue_ms.get(), tele.dsp_load.get() * 100.0,
        tele.underruns.load(Ordering::Relaxed), tele.overruns.load(Ordering::Relaxed), tele.restarts.load(Ordering::Relaxed)
    );
    host.stop();
    drop(duck_guard); // restores the original app volume
    println!("{summary}");
    if let Some((i, o)) = record {
        println!("A/B recordings: {} and {}", i.display(), o.display());
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use super::Args;
    use anyhow::{bail, Context, Result};
    use vo_audio::backend::{InputFactory, OutputFactory};
    use vo_audio::windows as w;
    use vo_audio::{CaptureSource, OutputTarget};

    pub fn apps() -> Result<()> {
        let apps = w::list_audio_apps()?;
        if apps.is_empty() {
            println!("No application is playing audio right now. Start the stream, then run this again.");
        }
        println!("{:<8} {:<28} {:>8} {:>7}  state", "PID", "application", "sessions", "peak");
        for a in apps {
            println!("{:<8} {:<28} {:>8} {:>7.3}  {}", a.pid, a.exe, a.sessions, a.peak, if a.active { "playing" } else { "idle" });
        }
        Ok(())
    }

    pub fn devices() -> Result<()> {
        println!("Output devices:");
        for d in w::list_output_devices()? {
            println!("  {} {}", if d.is_default { "*" } else { " " }, d.name);
        }
        println!("Input devices (usable with --input, e.g. VB-CABLE):");
        for d in w::list_input_devices()? {
            println!("  {} {}", if d.is_default { "*" } else { " " }, d.name);
        }
        Ok(())
    }

    pub fn beacon() -> Result<()> {
        let secs = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(8.0);
        w::latency::run_beacon(secs)
    }


    pub fn latency(a: &Args) -> Result<()> {
        let out = match &a.output {
            Some(q) => {
                let d = w::find_device(wasapi::Direction::Render, q)?.with_context(|| format!("no output device matches '{q}'"))?;
                OutputTarget { id: Some(d.id), name: d.name }
            }
            None => OutputTarget { id: None, name: "default output".into() },
        };
        let engine = if a.engine == "dfn3" { vo_dsp::EngineKind::Dfn3 } else { super::dpdfnet_kind().unwrap_or(vo_dsp::EngineKind::Dfn3) };
        println!("Measuring end-to-end latency to {} with {} (you will hear ~10 short beeps)…", out.name, engine.label());
        let exe = std::env::current_exe()?;
        let r = w::latency::measure(&exe, engine, out, a.buffer_ms, 8.0)?;
        println!(
            "Measured latency: median {:.0} ms (min {:.0}, max {:.0}, {} bursts) with {}. Excludes the DAC/headphone delay (Bluetooth adds more).",
            r.median_ms, r.min_ms, r.max_ms, r.samples_ms.len(), r.engine
        );
        Ok(())
    }

    pub fn endpoints(a: &Args) -> Result<(InputFactory, OutputFactory, String, Option<Box<dyn std::any::Any>>)> {
        let _ = w::duck::recover();
        let src = match (&a.app, &a.input) {
            (Some(app), None) => {
                let found = w::find_app(app)?.with_context(|| format!("no running application matches '{app}' (see `voiceolator-cli apps`)"))?;
                CaptureSource::Process { pid: found.pid, name: found.exe }
            }
            (None, Some(inp)) => {
                let d = w::find_device(wasapi::Direction::Capture, inp)?.with_context(|| format!("no input device matches '{inp}'"))?;
                CaptureSource::InputDevice { id: d.id, name: d.name }
            }
            _ => bail!("specify exactly one of --app NAME or --input DEVICE"),
        };
        let out = match &a.output {
            Some(q) => {
                let d = w::find_device(wasapi::Direction::Render, q)?.with_context(|| format!("no output device matches '{q}'"))?;
                OutputTarget { id: Some(d.id), name: d.name }
            }
            None => OutputTarget { id: None, name: "default output".into() },
        };
        // Feedback protection for device capture: never play into the device we record.
        // (VB-CABLE: "CABLE Input" is the playback side of "CABLE Output".)
        if let CaptureSource::InputDevice { name, .. } = &src {
            if name.to_lowercase().contains("cable") && out.name.to_lowercase().contains("cable") {
                bail!("output '{}' feeds the capture device '{}': this would create a feedback loop", out.name, name);
            }
        }
        let desc = format!("{} → {}", src.describe(), out.name);
        let output = w::output_factory(out, a.buffer_ms.max(10.0));
        match (&src, a.mute_original) {
            (CaptureSource::Process { name, .. }, true) => {
                // Hide the original: turn the app down in the mixer and compensate in capture.
                let target = w::DuckTarget::Exe(name.clone());
                let (input, duck) = w::ducked_input_factory(src, target);
                Ok((input, output, format!("{desc} (original sound hidden; --keep-original to hear it)"), Some(Box::new(duck))))
            }
            _ => Ok((w::input_factory(src), output, desc, None)),
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Args;
    use anyhow::{bail, Context, Result};
    use vo_audio::backend::{InputFactory, OutputFactory};
    use vo_audio::linux as l;
    use vo_audio::OutputTarget;

    pub fn beacon() -> Result<()> {
        let secs = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(8.0);
        l::run_beacon(secs)
    }

    pub fn apps() -> Result<()> {
        let apps = l::list_audio_apps()?;
        if apps.is_empty() {
            println!("No application is playing audio right now. Start the stream, then run this again.");
        }
        println!("{:<28} {:>7}  state", "application", "streams");
        for a in apps {
            println!("{:<28} {:>7}  {}", a.exe, a.streams, if a.playing { "playing" } else { "paused" });
        }
        Ok(())
    }

    pub fn devices() -> Result<()> {
        println!("Output devices:");
        for d in l::list_output_devices()? {
            println!("  {} {}", if d.is_default { "*" } else { " " }, d.name);
        }
        Ok(())
    }

    fn output(a: &Args) -> Result<OutputTarget> {
        Ok(match &a.output {
            Some(q) => {
                let d = l::find_output(q)?.with_context(|| format!("no output device matches '{q}'"))?;
                OutputTarget { id: Some(d.id), name: d.name }
            }
            None => OutputTarget { id: None, name: "default output".into() },
        })
    }

    pub fn latency(a: &Args) -> Result<()> {
        let out = output(a)?;
        let engine = if a.engine == "dfn3" { vo_dsp::EngineKind::Dfn3 } else { super::dpdfnet_kind().unwrap_or(vo_dsp::EngineKind::Dfn3) };
        println!("Measuring end-to-end latency to {} with {} (you will hear ~10 short beeps)…", out.name, engine.label());
        let exe = std::env::current_exe()?;
        let r = l::measure_latency(&exe, engine, out, a.buffer_ms, 8.0)?;
        println!(
            "Measured latency: median {:.0} ms (min {:.0}, max {:.0}, {} bursts) with {}. Excludes the DAC/headphone delay (Bluetooth adds more).",
            r.median_ms, r.min_ms, r.max_ms, r.samples_ms.len(), r.engine
        );
        Ok(())
    }

    pub fn endpoints(a: &Args) -> Result<(InputFactory, OutputFactory, String, Option<Box<dyn std::any::Any>>)> {
        let _ = l::recover();
        let Some(app) = &a.app else { bail!("specify --app NAME (see `voiceolator-cli apps`)") };
        let found = l::find_app(app)?.with_context(|| format!("no application matching '{app}' is playing audio (see `voiceolator-cli apps`)"))?;
        let out = output(a)?;
        let desc = format!("app {} → {}", found.exe, out.name);
        let (input, route) = l::routed_input_factory(l::RouteTarget::App(found.exe.clone()), l::CAPTURE_SINK, !a.mute_original);
        let note = if a.mute_original { " (original sound routed away; --keep-original to hear it)" } else { "" };
        Ok((input, l::output_factory(out, a.buffer_ms.max(10.0)), format!("{desc}{note}"), Some(Box::new(route))))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::Args;
    use anyhow::{bail, Result};
    use vo_audio::backend::{InputFactory, OutputFactory};
    pub fn beacon() -> Result<()> {
        bail!("not supported on this platform")
    }
    pub fn latency(_a: &Args) -> Result<()> {
        bail!("not supported on this platform")
    }
    pub fn apps() -> Result<()> {
        bail!("live capture is implemented for Windows and Linux only; use vo-eval for files")
    }
    pub fn devices() -> Result<()> {
        apps()
    }
    pub fn endpoints(_a: &Args) -> Result<(InputFactory, OutputFactory, String, Option<Box<dyn std::any::Any>>)> {
        bail!("live capture is implemented for Windows and Linux only; use vo-eval for files")
    }
}
