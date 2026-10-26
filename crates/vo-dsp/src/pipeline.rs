//! The complete per-hop processing chain:
//!
//! ```text
//! input ─┬─► VoiceEngine ─► attenuation limit ─► speech gate ─┬─► [high-pass ─► presence EQ ─► leveler] ─┐
//!        │                       ▲                            │                                        │
//!        └─► delay line ─────────┴────────────────────────────┴─ dry mix (during speech only)          │
//!                    │                                                                   ▼
//!                    └──────────────── bypass crossfade (isolation off) ─► volume ─► limiter ─► out
//! ```
//!
//! Everything here is allocation-free after construction and runs on the processing
//! thread, one hop (10 ms at 48 kHz for DFN3) at a time.

use crate::engine::{FrameInfo, VoiceEngine};
use crate::filters::{db_to_lin, lin_to_db, smoothing_coef, Biquad, DelayLine, Limiter};
use crate::settings::{PostParams, Settings};
use anyhow::Result;

/// Statistics for one processed hop (used for meters and telemetry).
#[derive(Clone, Copy, Debug, Default)]
pub struct HopStats {
    pub input_peak_db: f32,
    pub output_peak_db: f32,
    pub engine_snr_db: f32,
    /// 0..1, how open the speech gate is.
    pub speech_activity: f32,
    pub leveler_gain_db: f32,
    pub limiter_gain_db: f32,
}

const LEVELER_TARGET_DB: f32 = -20.0; // RMS target for speech
const LEVELER_MAX_BOOST_DB: f32 = 12.0;
const LEVELER_MAX_CUT_DB: f32 = -9.0;

pub struct Pipeline {
    engine: Box<dyn VoiceEngine>,
    settings: Settings,
    post: PostParams,
    sr: f32,
    hop: usize,
    hop_rate: f32,

    enh: Vec<f32>,
    dry: Vec<f32>,
    dry_delay: DelayLine,

    // speech gate
    hold_left: f32, // ms
    speech_ref_db: f32,
    gate: f32,      // current linear gate gain (per-sample ramped)
    speech_open: f32, // 0..1 smoothed "speech present"

    hp: Biquad,
    presence: Biquad,

    // leveler
    lev_power: f32,
    lev_gain_db: f32,
    lev_gain: f32,

    // bypass crossfade: 1 = fully processed
    wet: f32,
    out_gain: f32,
    limiter: Limiter,
}

impl Pipeline {
    pub fn new(engine: Box<dyn VoiceEngine>, settings: Settings) -> Self {
        let sr = engine.sample_rate() as f32;
        let hop = engine.hop_size();
        let latency = engine.latency_samples();
        let mut p = Self {
            hop,
            sr,
            hop_rate: sr / hop as f32,
            enh: vec![0.0; hop],
            dry: vec![0.0; hop],
            dry_delay: DelayLine::new(latency),
            hold_left: 0.0,
            speech_ref_db: -40.0,
            gate: 1.0,
            speech_open: 0.0,
            hp: Biquad::identity(),
            presence: Biquad::identity(),
            lev_power: db_to_lin(LEVELER_TARGET_DB).powi(2),
            lev_gain_db: 0.0,
            lev_gain: 1.0,
            wet: if settings.enabled { 1.0 } else { 0.0 },
            out_gain: settings.post_params().output_gain,
            limiter: Limiter::new(sr, -1.0, 80.0),
            post: settings.post_params(),
            settings,
            engine,
        };
        p.apply_settings(settings);
        p
    }

    pub fn engine_name(&self) -> &str {
        self.engine.name()
    }
    pub fn sample_rate(&self) -> usize {
        self.sr as usize
    }
    pub fn hop_size(&self) -> usize {
        self.hop
    }
    /// Algorithmic latency of the processing chain in samples (excludes buffering).
    pub fn latency_samples(&self) -> usize {
        self.engine.latency_samples()
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn apply_settings(&mut self, s: Settings) {
        self.settings = s;
        self.post = s.post_params();
        self.engine.set_params(&s.engine_params());
        if s.tone_shaping {
            // 2nd-order (12 dB/oct) rumble / bass-music cut, kept below the fundamental of
            // low male voices (~85 Hz) at default clarity.
            self.hp.set_highpass(self.sr, self.post.highpass_hz, 0.707);
            self.presence.set_peaking(self.sr, 3200.0, 0.8, self.post.presence_gain_db);
        } else {
            self.hp = Biquad::identity();
            self.presence = Biquad::identity();
        }
    }

    /// Process exactly one hop of mono audio.
    pub fn process_hop(&mut self, input: &[f32], output: &mut [f32]) -> Result<HopStats> {
        assert_eq!(input.len(), self.hop);
        assert_eq!(output.len(), self.hop);
        let p = self.post;

        let in_peak = input.iter().fold(0f32, |m, x| m.max(x.abs()));
        for (d, &x) in self.dry.iter_mut().zip(input) {
            *d = self.dry_delay.tick(x);
        }

        let FrameInfo { snr_db } = self.engine.process(input, &mut self.enh)?;

        // ---- engine-independent SNR estimate (for engines that don't report one) ----
        let e_enh = self.enh.iter().map(|x| x * x).sum::<f32>() / self.hop as f32;
        let e_dry = self.dry.iter().map(|x| x * x).sum::<f32>() / self.hop as f32;
        let snr_db = if snr_db.is_finite() {
            snr_db
        } else if e_dry < 1e-12 {
            -15.0
        } else {
            (10.0 * (e_enh / (e_dry - e_enh).max(e_dry * 1e-4)).log10()).clamp(-15.0, 35.0)
        };

        // ---- attenuation limit: never remove more than atten_limit_db of anything ----
        if p.atten_limit_db.is_finite() {
            let a = db_to_lin(-p.atten_limit_db);
            for (e, d) in self.enh.iter_mut().zip(&self.dry) {
                *e = *e * (1.0 - a) + d * a;
            }
        }

        // ---- speech-presence detection ----
        // Two cues, both taken from what the model did with this frame:
        //  * retention: how much of the frame's energy the model kept. Voice is kept,
        //    music/effects are removed, so a high ratio means "voice here".
        //  * level: model output within 15 dB of the recent voice level. This catches
        //    quiet voice under loud music, where retention is low even for voice (an
        //    input-SNR based detector cut 45 % of such speech in our tests).
        // The voice reference level only learns from frames the retention cue confirms,
        // so leftover music in a long music-only stretch can't become the reference.
        let hop_ms = 1000.0 / self.hop_rate;
        let enh_rms = e_enh.sqrt();
        let enh_db = lin_to_db(enh_rms);
        let retention_db = if e_dry > 1e-12 { 10.0 * (e_enh / e_dry).max(1e-9).log10() } else { -90.0 };
        let confident = retention_db > -8.0 && enh_db > -60.0;
        if confident && enh_db > self.speech_ref_db - 10.0 {
            let a = smoothing_coef(150.0, self.hop_rate);
            self.speech_ref_db = enh_db + (self.speech_ref_db - enh_db) * a;
        } else {
            self.speech_ref_db = (self.speech_ref_db - 1.0 * hop_ms / 1000.0).max(-45.0);
        }
        let speech_now = confident || (enh_db > self.speech_ref_db - 15.0 && enh_db > -55.0);
        if speech_now {
            self.hold_left = p.gate_hold_ms;
        } else {
            self.hold_left = (self.hold_left - hop_ms).max(0.0);
        }
        let open = speech_now || self.hold_left > 0.0;
        let floor = db_to_lin(-p.gate_depth_db);
        let target = if open { 1.0 } else { floor };
        // fast attack (3 ms), configurable release
        let coef = if target > self.gate {
            smoothing_coef(3.0, self.sr)
        } else {
            smoothing_coef(p.gate_release_ms, self.sr)
        };
        let o = if open { 1.0 } else { 0.0 };
        let open_coef = smoothing_coef(if open { 5.0 } else { p.gate_release_ms }, self.hop_rate);
        self.speech_open = o + (self.speech_open - o) * open_coef;

        // ---- leveler update (only on speech frames) ----
        if p.leveler && self.settings.tone_shaping && confident && enh_db > self.speech_ref_db - 12.0 {
            let pw = enh_rms * enh_rms;
            let a = smoothing_coef(1500.0, self.hop_rate);
            self.lev_power = pw + (self.lev_power - pw) * a;
            let rms_db = lin_to_db(self.lev_power.sqrt());
            self.lev_gain_db = (LEVELER_TARGET_DB - rms_db).clamp(LEVELER_MAX_CUT_DB, LEVELER_MAX_BOOST_DB);
        } else if !p.leveler || !self.settings.tone_shaping {
            self.lev_gain_db = 0.0;
        }
        let lev_target = db_to_lin(self.lev_gain_db);
        let lev_coef = smoothing_coef(200.0, self.sr);

        let wet_target = if self.settings.enabled { 1.0 } else { 0.0 };
        let wet_coef = smoothing_coef(15.0, self.sr);
        let gain_coef = smoothing_coef(20.0, self.sr);
        let dry_mix = p.dry_mix_in_speech * self.speech_open;

        let mut out_peak = 0f32;
        for i in 0..self.hop {
            self.gate = target + (self.gate - target) * coef;
            let dry = self.dry[i];
            let mut y = self.enh[i] * self.gate + dry * dry_mix;
            y = self.hp.process(y);
            y = self.presence.process(y);
            self.lev_gain = lev_target + (self.lev_gain - lev_target) * lev_coef;
            y *= self.lev_gain;

            self.wet = wet_target + (self.wet - wet_target) * wet_coef;
            let mixed = y * self.wet + dry * (1.0 - self.wet);
            self.out_gain = p.output_gain + (self.out_gain - p.output_gain) * gain_coef;
            let o = self.limiter.process(mixed * self.out_gain);
            out_peak = out_peak.max(o.abs());
            output[i] = o;
        }

        Ok(HopStats {
            input_peak_db: lin_to_db(in_peak),
            output_peak_db: lin_to_db(out_peak),
            engine_snr_db: snr_db,
            speech_activity: self.speech_open,
            leveler_gain_db: self.lev_gain_db,
            limiter_gain_db: self.limiter.gain_db(),
        })
    }
}

/// Convenience: run a whole buffer through a pipeline (offline), returning output aligned
/// to the input (engine latency removed) and per-hop stats.
pub fn process_offline(p: &mut Pipeline, input: &[f32]) -> Result<(Vec<f32>, Vec<HopStats>)> {
    let hop = p.hop_size();
    let lat = p.latency_samples();
    let total = input.len() + lat;
    let n_hops = total.div_ceil(hop);
    let mut padded = input.to_vec();
    padded.resize(n_hops * hop, 0.0);
    let mut out = vec![0.0; n_hops * hop];
    let mut stats = Vec::with_capacity(n_hops);
    for h in 0..n_hops {
        let r = h * hop..(h + 1) * hop;
        stats.push(p.process_hop(&padded[r.clone()], &mut out[r])?);
    }
    let aligned = out[lat..lat + input.len()].to_vec();
    Ok((aligned, stats))
}
