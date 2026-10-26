//! Lock-free telemetry shared between the audio threads and the UI/CLI.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// An f32 stored in an AtomicU32.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

#[derive(Default)]
pub struct Telemetry {
    pub running: AtomicBool,
    pub capture_ok: AtomicBool,
    pub output_ok: AtomicBool,
    pub engine_ok: AtomicBool,
    // meters (dBFS / 0..1)
    pub input_peak_db: AtomicF32,
    pub output_peak_db: AtomicF32,
    pub speech_activity: AtomicF32,
    // latency components (ms)
    pub capture_latency_ms: AtomicF32,
    pub input_queue_ms: AtomicF32,
    pub engine_latency_ms: AtomicF32,
    pub proc_ms_avg: AtomicF32,
    pub proc_ms_max: AtomicF32,
    pub output_queue_ms: AtomicF32,
    pub device_queue_ms: AtomicF32,
    /// Engine processing time / hop duration (1.0 = one full core-equivalent of real time).
    pub dsp_load: AtomicF32,
    pub drift_ppm: AtomicF32,
    /// Current adaptive jitter-buffer target (ms).
    pub jitter_target_ms: AtomicF32,
    // counters
    pub hops_processed: AtomicU64,
    /// Output dropouts caused by late audio (glitches).
    pub underruns: AtomicU64,
    /// Times the source stopped delivering audio for a while (paused/silent app).
    pub source_gaps: AtomicU64,
    pub overruns: AtomicU64,
    pub restarts: AtomicU64,
}

impl Telemetry {
    /// Current end-to-end latency estimate (ms): every buffer between the app's audio
    /// engine and the output device, plus the model's algorithmic delay.
    pub fn total_latency_ms(&self) -> f32 {
        self.capture_latency_ms.get()
            + self.input_queue_ms.get()
            + self.engine_latency_ms.get()
            + self.proc_ms_avg.get()
            + self.output_queue_ms.get()
            + self.device_queue_ms.get()
    }
}
