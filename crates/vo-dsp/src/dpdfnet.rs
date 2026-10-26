//! DPDFNet engine (Ceva, 2025) running on ONNX Runtime.
//!
//! DPDFNet extends DeepFilterNet2 with dual-path RNN blocks. In our evaluation
//! (see docs/EVALUATION.md) the 48 kHz `dpdfnet2_48khz_hr` variant clearly beats DFN3 on
//! speech-over-music: fewer cut syllables and stronger music removal, at ~2.4 GMAC/s.
//!
//! Model: `dpdfnet2_48khz_hr.onnx` (Apache-2.0, https://huggingface.co/Ceva-IP/DPDFNet).
//! Streaming signature: `spec [1,1,481,2] + state_in [N] -> spec_e [1,1,481,2] + state_out [N]`,
//! one call per 10 ms hop. STFT: 960-point Vorbis window, 480 hop, no centring (causal);
//! this mirrors `dpdfnet.StreamEnhancer` from the reference Python package.

use crate::engine::{EngineParams, FrameInfo, VoiceEngine};
use anyhow::{anyhow, Context, Result};
use ort::environment::Environment;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::TensorRef;
use realfft::{num_complex::Complex32, ComplexToReal, RealFftPlanner, RealToComplex};
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;

const WIN: usize = 960;
const HOP: usize = 480;
const BINS: usize = WIN / 2 + 1;
/// Measured: streaming output equals the input delayed by 1920 samples in the reference
/// implementation's indexing, plus one hop because we process the hop that just arrived
/// together with the previous one (see `latency_from_reference` test).
const LATENCY: usize = 2400;

static ORT_ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();

/// Load the ONNX Runtime shared library once per process (first caller's path wins).
pub fn init_onnxruntime(lib: &Path) -> Result<&'static Environment> {
    let r = ORT_ENV.get_or_init(|| {
        let b = ort::init_from(lib).map_err(|e| format!("could not load ONNX Runtime from {}: {e}", lib.display()))?;
        b.with_name("voiceolator").build().map_err(|e| format!("ONNX Runtime init failed: {e}"))
    });
    r.as_ref().map_err(|e| anyhow!(e.clone()))
}

fn vorbis_window() -> Vec<f32> {
    let h = WIN as f64 / 2.0;
    (0..WIN)
        .map(|i| {
            let s = (0.5 * std::f64::consts::PI * (i as f64 + 0.5) / h).sin();
            (0.5 * std::f64::consts::PI * s * s).sin() as f32
        })
        .collect()
}

pub struct DpdfnetEngine {
    session: Session,
    state: Vec<f32>,
    init_state: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    ifft: Arc<dyn ComplexToReal<f32>>,
    in_buf: Vec<f32>, // last WIN input samples
    frame: Vec<f32>,
    spec: Vec<Complex32>,
    spec_ri: Vec<f32>,
    ola: Vec<f32>,
    scratch_f: Vec<Complex32>,
    scratch_i: Vec<Complex32>,
    name: String,
}

impl DpdfnetEngine {
    /// `model`: path to a DPDFNet 48 kHz streaming ONNX file. ONNX Runtime must have been
    /// loaded with [`init_onnxruntime`] first (pass the returned environment).
    pub fn new(env: &Environment, model: &Path, _params: EngineParams) -> Result<Self> {
        let session = Session::builder(env)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("{e}"))?
            .with_intra_threads(1)
            .map_err(|e| anyhow!("{e}"))?
            .with_inter_threads(1)
            .map_err(|e| anyhow!("{e}"))?
            // Don't let idle ORT threads spin and steal CPU from the game.
            .with_intra_op_spinning(false)
            .map_err(|e| anyhow!("{e}"))?
            .commit_from_file(model)
            .with_context(|| format!("loading model {}", model.display()))?;

        let meta = session.metadata()?;
        let get = |k: &str| meta.custom(k).with_context(|| format!("model metadata '{k}' missing"));
        let sr: usize = get("sample_rate")?.parse()?;
        let n_fft: usize = get("n_fft")?.parse()?;
        let hop: usize = get("hop_length")?.parse()?;
        if sr != 48000 || n_fft != WIN || hop != HOP {
            anyhow::bail!("unsupported DPDFNet variant: sr={sr} n_fft={n_fft} hop={hop} (need 48000/960/480)");
        }
        let state_size: usize = get("state_size")?.parse()?;
        let erb_n: usize = get("erb_norm_state_size")?.parse()?;
        let spec_n: usize = get("spec_norm_state_size")?.parse()?;
        let parse = |s: String| -> Result<Vec<f32>> { s.split(',').map(|v| Ok(v.trim().parse::<f32>()?)).collect() };
        let erb_init = parse(get("erb_norm_init")?)?;
        let spec_init = parse(get("spec_norm_init")?)?;
        let profile = meta.custom("profile").unwrap_or_else(|| "dpdfnet".into());
        drop(meta);
        let mut init_state = vec![0.0; state_size];
        init_state[..erb_n].copy_from_slice(&erb_init[..erb_n]);
        init_state[erb_n..erb_n + spec_n].copy_from_slice(&spec_init[..spec_n]);

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(WIN);
        let ifft = planner.plan_fft_inverse(WIN);
        let scratch_f = fft.make_scratch_vec();
        let scratch_i = ifft.make_scratch_vec();
        // warm-up run so the first real frame doesn't pay for lazy initialisation
        let mut e = Self {
            session,
            state: init_state.clone(),
            init_state,
            window: vorbis_window(),
            fft,
            ifft,
            in_buf: vec![0.0; WIN],
            frame: vec![0.0; WIN],
            spec: vec![Complex32::new(0.0, 0.0); BINS],
            spec_ri: vec![0.0; BINS * 2],
            ola: vec![0.0; WIN],
            scratch_f,
            scratch_i,
            name: format!("DPDFNet ({profile})"),
        };
        let mut tmp = [0.0f32; HOP];
        e.process(&[0.0; HOP], &mut tmp)?;
        e.reset();
        Ok(e)
    }

    pub fn reset(&mut self) {
        self.state.copy_from_slice(&self.init_state);
        self.in_buf.fill(0.0);
        self.ola.fill(0.0);
    }
}

impl VoiceEngine for DpdfnetEngine {
    fn name(&self) -> &str {
        &self.name
    }
    fn sample_rate(&self) -> usize {
        48000
    }
    fn hop_size(&self) -> usize {
        HOP
    }
    fn latency_samples(&self) -> usize {
        LATENCY
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<FrameInfo> {
        debug_assert_eq!(input.len(), HOP);
        // slide input window
        self.in_buf.copy_within(HOP.., 0);
        self.in_buf[WIN - HOP..].copy_from_slice(input);
        for ((f, x), w) in self.frame.iter_mut().zip(&self.in_buf).zip(&self.window) {
            *f = x * w;
        }
        self.fft
            .process_with_scratch(&mut self.frame, &mut self.spec, &mut self.scratch_f)
            .map_err(|e| anyhow!("fft: {e}"))?;
        for (i, c) in self.spec.iter().enumerate() {
            self.spec_ri[2 * i] = c.re;
            self.spec_ri[2 * i + 1] = c.im;
        }

        {
            let spec_t = TensorRef::from_array_view(([1usize, 1, BINS, 2], &self.spec_ri[..]))?;
            let state_t = TensorRef::from_array_view(([self.state.len()], &self.state[..]))?;
            let out = self.session.run(ort::inputs![spec_t, state_t])?;
            let (_, spec_e) = out[0].try_extract_tensor::<f32>()?;
            let (_, st) = out[1].try_extract_tensor::<f32>()?;
            for (i, c) in self.spec.iter_mut().enumerate() {
                *c = Complex32::new(spec_e[2 * i], spec_e[2 * i + 1]);
            }
            self.state.copy_from_slice(st);
        }
        // c2r requires purely real DC and Nyquist bins
        self.spec[0].im = 0.0;
        self.spec[BINS - 1].im = 0.0;
        self.ifft
            .process_with_scratch(&mut self.spec, &mut self.frame, &mut self.scratch_i)
            .map_err(|e| anyhow!("ifft: {e}"))?;
        let norm = 1.0 / WIN as f32;
        for ((o, x), w) in self.ola.iter_mut().zip(&self.frame).zip(&self.window) {
            *o += x * norm * w;
        }
        output.copy_from_slice(&self.ola[..HOP]);
        self.ola.copy_within(HOP.., 0);
        self.ola[WIN - HOP..].fill(0.0);
        Ok(FrameInfo { snr_db: f32::NAN })
    }

    fn set_params(&mut self, _p: &EngineParams) {
        // DPDFNet has no internal knobs; attenuation limiting is done by the pipeline.
    }
}
