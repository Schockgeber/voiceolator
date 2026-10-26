//! DeepFilterNet3 engine (via the reference `deep_filter` crate, tract backend).
//!
//! Model: DeepFilterNet3, 48 kHz full-band, 10 ms hop, 20 ms window, 2-frame lookahead.
//! License: MIT OR Apache-2.0 (Hendrik Schröter). The model weights are embedded in the
//! binary by the `default-model` feature, so no model file has to be installed.

use crate::engine::{EngineParams, FrameInfo, VoiceEngine};
use anyhow::Result;
use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};

/// libDF skips the model for frames whose estimated SNR is above these thresholds. With the
/// library defaults (30/20 dB) we measured audible damage on clean speech (SI-SDR 5.8 dB vs
/// 19.7 dB with 35/35, which is what the upstream CLI uses), so we follow the CLI.
const MAX_DB_THRESH: f32 = 35.0;

pub struct DfnEngine {
    model: DfTract,
    params: EngineParams,
    latency: usize,
}

impl DfnEngine {
    pub fn new(params: EngineParams) -> Result<Self> {
        let rp = RuntimeParams::default_with_ch(1)
            .with_atten_lim(params.atten_limit_db)
            .with_post_filter(params.post_filter)
            .with_thresholds(params.min_speech_snr_db, MAX_DB_THRESH, MAX_DB_THRESH);
        let model = DfTract::new(DfParams::default(), &rp)?;
        // STFT delay (window - hop) + model lookahead, see libDF enhance_wav.rs.
        let latency = model.fft_size - model.hop_size + model.lookahead * model.hop_size;
        let mut e = Self { model, params, latency };
        e.set_params(&params);
        Ok(e)
    }
}

impl VoiceEngine for DfnEngine {
    fn name(&self) -> &str {
        "DeepFilterNet3"
    }
    fn sample_rate(&self) -> usize {
        self.model.sr
    }
    fn hop_size(&self) -> usize {
        self.model.hop_size
    }
    fn latency_samples(&self) -> usize {
        self.latency
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<FrameInfo> {
        let hop = self.model.hop_size;
        let inp = ArrayView2::from_shape((1, hop), input)?;
        let out = ArrayViewMut2::from_shape((1, hop), output)?;
        let snr_db = self.model.process(inp, out)?;
        Ok(FrameInfo { snr_db })
    }

    fn set_params(&mut self, p: &EngineParams) {
        self.params = *p;
        self.model.set_atten_lim(p.atten_limit_db.clamp(0.0, 100.0));
        self.model.set_pf_beta(p.post_filter.clamp(0.0, 0.05));
        self.model.min_db_thresh = p.min_speech_snr_db;
    }
}
