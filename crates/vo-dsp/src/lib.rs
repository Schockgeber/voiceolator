//! Voiceolator DSP core (platform independent).
//!
//! * [`engine`]   – the swappable voice-model interface
//! * [`dfn`]      – DeepFilterNet3 engine (pure Rust/tract, model embedded)
//! * [`dpdfnet`]  – DPDFNet engine (ONNX Runtime, model file + runtime DLL)
//! * [`pipeline`] – engine + speech gate + clarity EQ + leveler + limiter
//! * [`settings`] – user sliders/presets and their parameter mapping
//! * [`drift`]    – adaptive resampler that absorbs capture/output clock drift
//! * [`filters`]  – small real-time-safe DSP helpers

pub mod dfn;
pub mod dpdfnet;
pub mod drift;
pub mod engine;
pub mod filters;
pub mod pipeline;
pub mod settings;

pub use engine::{EngineParams, FrameInfo, VoiceEngine};
pub use pipeline::{process_offline, HopStats, Pipeline};
pub use settings::{Preset, Settings};

use std::path::PathBuf;

/// Which voice model to run.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineKind {
    /// DPDFNet-2 48 kHz on ONNX Runtime. Best quality on speech-over-music (default).
    Dpdfnet { model: PathBuf, onnxruntime: PathBuf },
    /// DeepFilterNet3 (pure Rust, embedded). Lowest latency/CPU, weaker on music.
    Dfn3,
}

impl EngineKind {
    pub fn label(&self) -> &'static str {
        match self {
            EngineKind::Dpdfnet { .. } => "DPDFNet (quality)",
            EngineKind::Dfn3 => "DeepFilterNet3 (light)",
        }
    }
}

/// Construct an engine. Must be called on the thread that will run it (engines are !Send).
pub fn create_engine(kind: &EngineKind, settings: &Settings) -> anyhow::Result<Box<dyn VoiceEngine>> {
    Ok(match kind {
        EngineKind::Dpdfnet { model, onnxruntime } => {
            let env = dpdfnet::init_onnxruntime(onnxruntime)?;
            Box::new(dpdfnet::DpdfnetEngine::new(env, model, settings.engine_params())?)
        }
        EngineKind::Dfn3 => Box::new(dfn::DfnEngine::new(settings.engine_params())?),
    })
}

/// Create DeepFilterNet3 (no external files needed).
pub fn default_engine(settings: &Settings) -> anyhow::Result<Box<dyn VoiceEngine>> {
    create_engine(&EngineKind::Dfn3, settings)
}
