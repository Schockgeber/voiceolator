//! User-facing settings, presets and their mapping to engine/post-processor parameters.

use crate::engine::EngineParams;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Balanced,
    VoicePriority,
    MaximumSuppression,
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Balanced, Preset::VoicePriority, Preset::MaximumSuppression];
    pub fn label(self) -> &'static str {
        match self {
            Preset::Balanced => "Balanced",
            Preset::VoicePriority => "Voice Priority",
            Preset::MaximumSuppression => "Maximum Suppression",
            Preset::Custom => "Custom",
        }
    }
    pub fn parse(s: &str) -> Option<Preset> {
        match s.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "balanced" => Some(Preset::Balanced),
            "voicepriority" | "voice" => Some(Preset::VoicePriority),
            "maximumsuppression" | "max" | "maximum" => Some(Preset::MaximumSuppression),
            _ => None,
        }
    }
}

/// All user-adjustable processing settings. Slider values are normalised to 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Voice isolation on/off. Off = original audio (delay-matched) is passed through.
    pub enabled: bool,
    /// How strongly speech is enhanced (presence EQ, post-filter, rumble cut).
    pub clarity: f32,
    /// How aggressively music and effects are removed.
    pub suppression: f32,
    /// How much quiet or overlapping speech is protected from suppression.
    pub preservation: f32,
    /// Output gain, linear (1.0 = 0 dB). Applied after the limiter's makeup stage.
    pub volume: f32,
    pub mute: bool,
    /// Automatic level control: keep speech loudness consistent (optional).
    pub leveler: bool,
    /// Clarity EQ + leveler stage. Always on in the app; the evaluator turns it off to
    /// measure the separation itself (linear EQ/AGC distort SI-SDR without being audible
    /// as damage).
    pub tone_shaping: bool,
    pub preset: Preset,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_preset(Preset::Balanced)
    }
}

impl Settings {
    pub fn from_preset(p: Preset) -> Self {
        let (clarity, suppression, preservation) = match p {
            Preset::Balanced | Preset::Custom => (0.5, 0.7, 0.5),
            Preset::VoicePriority => (0.4, 0.45, 0.9),
            Preset::MaximumSuppression => (0.6, 1.0, 0.15),
        };
        Self {
            enabled: true,
            clarity,
            suppression,
            preservation,
            volume: 1.0,
            mute: false,
            // Off by default: it equalises whatever the model keeps, which includes sung
            // vocals during music-only stretches (measured +9 dB on such passages).
            leveler: false,
            tone_shaping: true,
            preset: p,
        }
    }

    /// Engine parameters implied by the sliders.
    pub fn engine_params(&self) -> EngineParams {
        let v = self.preservation.clamp(0.0, 1.0);
        let c = self.clarity.clamp(0.0, 1.0);
        EngineParams {
            // Attenuation limiting is done engine-independently in the pipeline.
            atten_limit_db: 100.0,
            // DFN3 only: frames below this estimated SNR are zeroed. More preservation ->
            // lower threshold -> quiet speech is less likely to be treated as noise.
            min_speech_snr_db: -10.0 - 10.0 * v,
            post_filter: 0.02 * c,
        }
    }

    pub fn post_params(&self) -> PostParams {
        let s = self.suppression.clamp(0.0, 1.0);
        let v = self.preservation.clamp(0.0, 1.0);
        let c = self.clarity.clamp(0.0, 1.0);
        PostParams {
            // 0 -> 12 dB, 0.5 -> ~30 dB, >= 0.9 -> unlimited (whatever the model removes)
            atten_limit_db: if s >= 0.9 { f32::INFINITY } else { 12.0 * (2.5f32).powf(2.0 * s) },
            // Extra gating between phrases only at the top of the range: the models already
            // remove 40-60 dB in pauses and the gate is what risks cutting quiet words.
            gate_depth_db: 12.0 * ((s - 0.7) / 0.3).clamp(0.0, 1.0) * (1.0 - v),
            gate_hold_ms: 150.0 + 350.0 * v,
            gate_release_ms: 80.0 + 170.0 * v,
            dry_mix_in_speech: 0.10 * v * v * (1.0 - 0.5 * s),
            highpass_hz: 50.0 + 40.0 * c,
            presence_gain_db: 5.0 * c,
            leveler: self.leveler,
            output_gain: if self.mute { 0.0 } else { self.volume.clamp(0.0, 4.0) },
        }
    }
}

/// Low-level parameters of the post-processor (derived from [`Settings`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostParams {
    /// Maximum total attenuation of non-speech (dB); implemented by mixing back the
    /// delay-aligned original at -atten_limit_db. `INFINITY` = no limit.
    pub atten_limit_db: f32,
    /// Extra attenuation applied while no speech is detected (dB).
    pub gate_depth_db: f32,
    /// How long the gate stays open after speech ends (ms) – protects word endings.
    pub gate_hold_ms: f32,
    /// Fade-out time after the hold period (ms) – avoids pumping.
    pub gate_release_ms: f32,
    /// Fraction of the original (delay-aligned) signal mixed back in during speech.
    /// Masks model artifacts on quiet/overlapped speech at the cost of some bleed.
    pub dry_mix_in_speech: f32,
    pub highpass_hz: f32,
    pub presence_gain_db: f32,
    pub leveler: bool,
    pub output_gain: f32,
}
