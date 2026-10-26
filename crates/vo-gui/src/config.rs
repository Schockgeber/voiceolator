//! Persisted user configuration (JSON in %APPDATA%\Voiceolator\config.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vo_dsp::{Preset, Settings};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineChoice {
    Quality,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Executable name of the last audio source, e.g. "firefox.exe".
    pub source_exe: Option<String>,
    /// Output device id (None = Windows default output).
    pub output_id: Option<String>,
    pub output_name: Option<String>,
    pub engine: EngineChoice,
    pub preset: String,
    pub clarity: f32,
    pub suppression: f32,
    pub preservation: f32,
    pub volume_db: f32,
    pub muted: bool,
    pub isolation_enabled: bool,
    /// Hide the source app's own playback while running (turned down + compensated).
    /// (Replaces 0.9.0's `mute_original`, which is ignored when loading old files.)
    pub hide_original: bool,
    pub leveler: bool,
    pub always_on_top: bool,
    pub start_automatically: bool,
    pub buffer_ms: f32,
    /// Mixer sessions we muted and have not yet restored (crash recovery).
    pub muted_sessions: Vec<String>,
    pub last_measured_latency_ms: Option<f64>,
}

impl Default for Config {
    fn default() -> Self {
        let s = Settings::default();
        Self {
            source_exe: None,
            output_id: None,
            output_name: None,
            engine: EngineChoice::Quality,
            preset: "balanced".into(),
            clarity: s.clarity,
            suppression: s.suppression,
            preservation: s.preservation,
            volume_db: 0.0,
            muted: false,
            isolation_enabled: true,
            hide_original: true,
            leveler: false,
            always_on_top: false,
            start_automatically: false,
            buffer_ms: 20.0,
            muted_sessions: Vec::new(),
            last_measured_latency_ms: None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("Voiceolator")
}

pub fn log_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("Voiceolator"));
    #[cfg(not(windows))]
    let base: Option<PathBuf> = None;
    base.unwrap_or_else(config_dir).join("voiceolator.log")
}

impl Config {
    pub fn load() -> Self {
        let p = config_dir().join("config.json");
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                log::warn!("config {} unreadable ({e}); using defaults", p.display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        let dir = config_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("cannot create {}: {e}", dir.display());
            return;
        }
        let tmp = dir.join("config.json.tmp");
        let res = serde_json::to_string_pretty(self)
            .map_err(anyhow::Error::from)
            .and_then(|s| std::fs::write(&tmp, s).map_err(Into::into))
            .and_then(|_| std::fs::rename(&tmp, dir.join("config.json")).map_err(Into::into));
        if let Err(e) = res {
            log::warn!("saving config failed: {e}");
        }
    }

    pub fn preset(&self) -> Preset {
        Preset::parse(&self.preset).unwrap_or(Preset::Custom)
    }

    pub fn settings(&self) -> Settings {
        let mut s = Settings::from_preset(self.preset());
        s.clarity = self.clarity;
        s.suppression = self.suppression;
        s.preservation = self.preservation;
        s.volume = 10f32.powf(self.volume_db / 20.0);
        s.mute = self.muted;
        s.enabled = self.isolation_enabled;
        s.leveler = self.leveler;
        s.preset = self.preset();
        s
    }

    pub fn apply_preset(&mut self, p: Preset) {
        let s = Settings::from_preset(p);
        self.clarity = s.clarity;
        self.suppression = s.suppression;
        self.preservation = s.preservation;
        self.preset = match p {
            Preset::Balanced => "balanced",
            Preset::VoicePriority => "voice",
            Preset::MaximumSuppression => "max",
            Preset::Custom => "custom",
        }
        .into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial_files() {
        let mut c = Config::default();
        c.source_exe = Some("firefox.exe".into());
        c.apply_preset(Preset::VoicePriority);
        let j = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&j).unwrap();
        assert_eq!(back.source_exe.as_deref(), Some("firefox.exe"));
        assert_eq!(back.preset(), Preset::VoicePriority);
        // Older/partial config files must load with defaults for missing fields.
        let partial: Config = serde_json::from_str(r#"{"source_exe":"chrome.exe","mute_original":false}"#).unwrap();
        assert!(partial.hide_original);
        assert_eq!(partial.engine, EngineChoice::Quality);
    }

    #[test]
    fn settings_follow_config() {
        let mut c = Config::default();
        c.apply_preset(Preset::MaximumSuppression);
        c.volume_db = -6.0;
        c.isolation_enabled = false;
        let s = c.settings();
        assert_eq!(s.preset, Preset::MaximumSuppression);
        assert!((s.volume - 0.501).abs() < 0.01);
        assert!(!s.enabled);
        c.preset = "custom".into();
        assert_eq!(c.settings().preset, Preset::Custom);
    }
}
