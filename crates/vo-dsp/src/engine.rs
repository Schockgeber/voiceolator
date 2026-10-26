//! Voice-engine abstraction.
//!
//! A [`VoiceEngine`] turns fixed-size frames ("hops") of mono 48 kHz audio into
//! enhanced (voice-only) frames. Everything model-specific lives behind this trait so
//! the model can be swapped (DeepFilterNet3 today; DPDFNet or a separation model later)
//! without touching capture, output, UI or the post-processor.

/// Per-frame information returned by an engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInfo {
    /// Engine's estimate of the local speech-to-noise ratio for this frame, in dB.
    /// Roughly: < -10 dB = no speech, > 20 dB = clean speech.
    pub snr_db: f32,
}

/// Engine controls derived from the user-facing sliders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineParams {
    /// Maximum attenuation the model may apply to non-speech, in dB (6..=100).
    /// 100 means "unlimited".
    pub atten_limit_db: f32,
    /// Frames whose estimated SNR is below this (dB) are treated as "no speech" and
    /// fully suppressed by the model. Lower = protects more quiet speech.
    pub min_speech_snr_db: f32,
    /// Strength of the model's perceptual post-filter (0 = off, 0.05 = strong).
    pub post_filter: f32,
}

impl Default for EngineParams {
    fn default() -> Self {
        Self { atten_limit_db: 100.0, min_speech_snr_db: -15.0, post_filter: 0.0 }
    }
}

/// Note: not `Send` — DeepFilterNet's tract state uses `Rc`. Engines are constructed on, and
/// never leave, the processing thread (see `vo-audio`'s engine factory).
pub trait VoiceEngine {
    /// Human-readable engine name, e.g. "DeepFilterNet3".
    fn name(&self) -> &str;
    /// Sample rate the engine operates at (Hz).
    fn sample_rate(&self) -> usize;
    /// Number of samples consumed/produced per [`process`](Self::process) call.
    fn hop_size(&self) -> usize;
    /// Algorithmic delay of the engine in samples: output sample `n` corresponds to
    /// input sample `n - latency_samples()`.
    fn latency_samples(&self) -> usize;
    /// Process one hop. `input.len() == output.len() == hop_size()`.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> anyhow::Result<FrameInfo>;
    /// Apply new parameters. Must be cheap; called from the processing thread.
    fn set_params(&mut self, params: &EngineParams);
}
