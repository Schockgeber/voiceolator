//! Platform glue for the GUI: audio sources, outputs, mute control, latency measurement.
//!
//! Windows uses the real WASAPI backend. Other platforms get a *demo* backend (a
//! synthetic "stream" played through simulated devices) so the UI can be developed and
//! tested anywhere; real capture for macOS/Linux would slot in here.

use crate::config::EngineChoice;
use anyhow::Result;
use std::path::{Path, PathBuf};
use vo_audio::backend::{InputFactory, OutputFactory};
use vo_dsp::EngineKind;

#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub exe: String,
    pub pid: u32,
    pub playing: bool,
}

impl SourceInfo {
    pub fn label(&self) -> String {
        let name = self.exe.trim_end_matches(".exe").trim_end_matches(".EXE");
        let pretty = match name.to_ascii_lowercase().as_str() {
            "firefox" => "Firefox".to_string(),
            "chrome" => "Google Chrome".to_string(),
            "msedge" => "Microsoft Edge".to_string(),
            "opera" | "opera_gx" => "Opera".to_string(),
            "brave" => "Brave".to_string(),
            "vivaldi" => "Vivaldi".to_string(),
            "twitch" => "Twitch app".to_string(),
            "vlc" => "VLC".to_string(),
            "mpv" => "mpv".to_string(),
            _ => name.to_string(),
        };
        if self.playing {
            format!("{pretty}  (playing)")
        } else {
            pretty
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputInfo {
    pub id: Option<String>,
    pub name: String,
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| ".".into())
}

/// Resolve the engine choice to an engine; Quality needs the model + ONNX Runtime files.
pub fn engine_kind(choice: EngineChoice) -> (EngineKind, Option<String>) {
    match choice {
        EngineChoice::Light => (EngineKind::Dfn3, None),
        EngineChoice::Quality => {
            let d = exe_dir();
            let model = d.join("models").join("dpdfnet2_48khz_hr.onnx");
            let ort = d.join(if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so" });
            if model.exists() && ort.exists() {
                (EngineKind::Dpdfnet { model, onnxruntime: ort }, None)
            } else {
                (EngineKind::Dfn3, Some("Quality engine files are missing (models\\dpdfnet2_48khz_hr.onnx / onnxruntime.dll); using the Light engine. Reinstall to fix.".into()))
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::Context;
    use vo_audio::windows as w;
    use vo_audio::{CaptureSource, OutputTarget};

    pub fn list_sources() -> Result<Vec<SourceInfo>> {
        Ok(w::list_audio_apps()?.into_iter().map(|a| SourceInfo { exe: a.exe, pid: a.pid, playing: a.active && a.peak > 0.0001 }).collect())
    }

    pub fn list_outputs() -> Result<Vec<OutputInfo>> {
        let mut v = vec![OutputInfo { id: None, name: "Windows default output".into() }];
        v.extend(w::list_output_devices()?.into_iter().map(|d| OutputInfo { id: Some(d.id), name: d.name }));
        Ok(v)
    }

    /// Keeps the original app sound hidden while alive; restores it when dropped.
    pub struct HideGuard(std::sync::Arc<w::DuckShared>);
    impl HideGuard {
        pub fn restore(&self) {
            self.0.restore();
        }
        pub fn compensated(&self) -> Option<bool> {
            match self.0.mode() {
                vo_audio::duckcomp::DuckMode::Undecided => None,
                vo_audio::duckcomp::DuckMode::Compensated => Some(true),
                vo_audio::duckcomp::DuckMode::Unity => Some(false),
            }
        }
    }

    pub fn factories(exe: &str, out: &OutputInfo, buffer_ms: f32, hide_original: bool) -> Result<(InputFactory, OutputFactory, Option<HideGuard>)> {
        // pid 0 = resolve by name on every (re)connect, so browser restarts are followed.
        let pid = w::find_app(exe)?.map(|a| a.pid).unwrap_or(0);
        let src = CaptureSource::Process { pid, name: exe.to_string() };
        let target = OutputTarget { id: out.id.clone(), name: out.name.clone() };
        let output = w::output_factory(target, buffer_ms.max(10.0));
        if hide_original {
            let (input, duck) = w::ducked_input_factory(src, w::DuckTarget::Exe(exe.to_string()));
            Ok((input, output, Some(HideGuard(duck))))
        } else {
            Ok((w::input_factory(src), output, None))
        }
    }

    /// Undo anything a previous run left behind (crash): turned-down or muted sessions.
    pub fn recover(legacy_muted: &[String]) -> usize {
        w::duck::recover().unwrap_or(0) + w::unmute_session_ids(legacy_muted).unwrap_or(0)
    }

    pub fn app_running(exe: &str) -> bool {
        w::find_app(exe).ok().flatten().is_some()
    }


    pub fn measure_latency(engine: EngineKind, out: &OutputInfo, buffer_ms: f32) -> Result<vo_audio::windows::latency::LatencyReport> {
        let exe = std::env::current_exe().context("own path")?;
        let target = OutputTarget { id: out.id.clone(), name: out.name.clone() };
        w::latency::measure(&exe, engine, target, buffer_ms.max(10.0), 8.0)
    }

    pub fn run_beacon(secs: f64) -> Result<()> {
        w::latency::run_beacon(secs)
    }

    pub fn demo_mode() -> bool {
        false
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
mod demo {
    //! Demo backend: a synthetic voice-like signal over music-like chords, through
    //! simulated real-time devices.
    use super::*;
    use parking_lot::Mutex;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use vo_audio::testing::{SimInput, SimOutput};

    pub struct LatencyReport {
        pub median_ms: f64,
        pub min_ms: f64,
        pub max_ms: f64,
        pub samples_ms: Vec<f64>,
        pub engine: String,
    }

    pub fn list_sources() -> Result<Vec<SourceInfo>> {
        Ok(vec![
            SourceInfo { exe: "firefox.exe".into(), pid: 1, playing: true },
            SourceInfo { exe: "chrome.exe".into(), pid: 2, playing: false },
        ])
    }

    pub fn list_outputs() -> Result<Vec<OutputInfo>> {
        Ok(vec![
            OutputInfo { id: None, name: "Default output (simulated)".into() },
            OutputInfo { id: Some("hs".into()), name: "Headset Earphone (simulated)".into() },
            OutputInfo { id: Some("spk".into()), name: "Speakers (simulated)".into() },
        ])
    }

    fn demo_signal() -> Vec<f32> {
        if let Some(p) = std::env::var_os("VOICEOLATOR_DEMO_WAV") {
            if let Ok(mut r) = hound::WavReader::open(p) {
                let ch = r.spec().channels as usize;
                let v: Vec<f32> = r.samples::<f32>().filter_map(|s| s.ok()).collect();
                return v.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
            }
        }
        (0..48000 * 120)
            .map(|i| {
                let t = i as f32 / 48000.0;
                let voice = if (t % 1.3) < 0.8 { 0.12 * (2.0 * std::f32::consts::PI * 160.0 * t).sin() } else { 0.0 };
                voice + 0.05 * (2.0 * std::f32::consts::PI * 220.0 * t).sin() + 0.04 * (2.0 * std::f32::consts::PI * 330.0 * t).sin()
            })
            .collect()
    }

    pub struct HideGuard;
    impl HideGuard {
        pub fn restore(&self) {}
        pub fn compensated(&self) -> Option<bool> {
            Some(true)
        }
    }

    pub fn recover(_legacy: &[String]) -> usize {
        0
    }

    pub fn factories(_exe: &str, _out: &OutputInfo, _buffer_ms: f32, hide_original: bool) -> Result<(InputFactory, OutputFactory, Option<HideGuard>)> {
        let sig = Arc::new(demo_signal());
        let pos = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(Mutex::new(Vec::new()));
        let input: InputFactory = Box::new(move || Ok(Box::new(SimInput::new(sig.clone(), pos.clone(), 1.0, None))));
        let output: OutputFactory = Box::new(move || {
            sink.lock().clear();
            Ok(Box::new(SimOutput::new(sink.clone(), 1.0)))
        });
        Ok((input, output, hide_original.then_some(HideGuard)))
    }

    pub fn app_running(exe: &str) -> bool {
        exe == "firefox.exe"
    }


    pub fn measure_latency(_e: EngineKind, _o: &OutputInfo, _b: f32) -> Result<LatencyReport> {
        anyhow::bail!("latency measurement needs real audio devices (Windows)")
    }

    pub fn run_beacon(_secs: f64) -> Result<()> {
        anyhow::bail!("Windows only")
    }

}

/// Linux: real PulseAudio/PipeWire backend; `VOICEOLATOR_DEMO=1` switches to the demo.
#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use anyhow::Context;
    use vo_audio::linux as l;
    use vo_audio::OutputTarget;

    pub fn demo_mode() -> bool {
        std::env::var_os("VOICEOLATOR_DEMO").is_some()
    }

    pub fn list_sources() -> Result<Vec<SourceInfo>> {
        if demo_mode() {
            return demo::list_sources();
        }
        Ok(l::list_audio_apps()?.into_iter().map(|a| SourceInfo { exe: a.exe, pid: a.pid, playing: a.playing }).collect())
    }

    pub fn list_outputs() -> Result<Vec<OutputInfo>> {
        if demo_mode() {
            return demo::list_outputs();
        }
        let mut v = vec![OutputInfo { id: None, name: "System default output".into() }];
        v.extend(l::list_output_devices()?.into_iter().map(|d| OutputInfo { id: Some(d.id), name: d.name }));
        Ok(v)
    }

    pub enum HideGuard {
        Routed(std::sync::Arc<l::RouteShared>),
        Demo,
    }
    impl HideGuard {
        pub fn restore(&self) {
            if let HideGuard::Routed(r) = self {
                r.restore();
            }
        }
        pub fn compensated(&self) -> Option<bool> {
            Some(true)
        }
    }

    /// The app's streams are moved into Voiceolator's private capture sink, so the
    /// original is never played. With `hide_original == false` a loopback plays it too.
    pub fn factories(exe: &str, out: &OutputInfo, buffer_ms: f32, hide_original: bool) -> Result<(InputFactory, OutputFactory, Option<HideGuard>)> {
        if demo_mode() {
            let (i, o, _) = demo::factories(exe, out, buffer_ms, hide_original)?;
            return Ok((i, o, Some(HideGuard::Demo)));
        }
        let target = OutputTarget { id: out.id.clone(), name: out.name.clone() };
        let (input, route) = l::routed_input_factory(l::RouteTarget::App(exe.to_string()), l::CAPTURE_SINK, !hide_original);
        Ok((input, l::output_factory(target, buffer_ms.max(10.0)), Some(HideGuard::Routed(route))))
    }

    pub fn recover(_legacy_muted: &[String]) -> usize {
        if demo_mode() {
            return 0;
        }
        l::recover().unwrap_or(0)
    }

    pub fn app_running(exe: &str) -> bool {
        if demo_mode() {
            return demo::app_running(exe);
        }
        l::find_app(exe).ok().flatten().is_some()
    }

    pub fn measure_latency(engine: EngineKind, out: &OutputInfo, buffer_ms: f32) -> Result<l::LatencyReport> {
        let exe = std::env::current_exe().context("own path")?;
        let target = OutputTarget { id: out.id.clone(), name: out.name.clone() };
        l::measure_latency(&exe, engine, target, buffer_ms.max(10.0), 8.0)
    }

    pub fn run_beacon(secs: f64) -> Result<()> {
        l::run_beacon(secs)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub use super::demo::*;
    pub fn demo_mode() -> bool {
        true
    }
}

pub use imp::*;
