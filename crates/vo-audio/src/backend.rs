//! Platform audio stream abstractions.
//!
//! Streams are created *on* the thread that uses them (COM objects on Windows are
//! thread-affine), so the host receives factories, not streams.

use anyhow::Result;

/// What to capture.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureSource {
    /// Capture everything one application (process tree) plays — WASAPI process loopback.
    /// `pid` must be the root process of the app (e.g. the main browser process).
    Process { pid: u32, name: String },
    /// Capture an input device, e.g. "CABLE Output (VB-Audio Virtual Cable)".
    InputDevice { id: String, name: String },
    /// Capture what an output device plays (classic loopback). Captures *all* apps on it.
    OutputLoopback { id: String, name: String },
}

impl CaptureSource {
    pub fn describe(&self) -> String {
        match self {
            CaptureSource::Process { pid, name } => format!("app {name} (pid {pid})"),
            CaptureSource::InputDevice { name, .. } => format!("input device {name}"),
            CaptureSource::OutputLoopback { name, .. } => format!("loopback of {name}"),
        }
    }
}

/// Where to play the processed voice. `None` id = system default output.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OutputTarget {
    pub id: Option<String>,
    pub name: String,
}

pub trait InputStream {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> usize;
    /// Frames of buffering inside the OS/driver before we see the data (estimate).
    fn latency_frames(&self) -> usize;
    /// Wait up to `timeout_ms` for data and append interleaved f32 frames to `out`.
    /// Returns the number of frames appended (0 = nothing yet / silence-timeout).
    fn read(&mut self, out: &mut Vec<f32>, timeout_ms: u32) -> Result<usize>;
}

pub trait OutputStream {
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> usize;
    /// Block until the device wants more data (or timeout).
    fn wait(&mut self, timeout_ms: u32) -> Result<()>;
    /// Frames that can be written now.
    fn writable_frames(&mut self) -> Result<usize>;
    /// Frames already queued in the device buffer (not yet played).
    fn queued_frames(&mut self) -> Result<usize>;
    /// Write interleaved frames (`data.len() == frames * channels`).
    fn write(&mut self, data: &[f32]) -> Result<()>;
    /// When the frame with this index (counted from stream start) is/was played, as a
    /// system performance-counter time in 100 ns units. Used for latency measurement.
    fn play_time_100ns(&mut self, _frame_index: u64) -> Option<i64> {
        None
    }
}

pub type InputFactory = Box<dyn Fn() -> Result<Box<dyn InputStream>> + Send + Sync>;
pub type OutputFactory = Box<dyn Fn() -> Result<Box<dyn OutputStream>> + Send + Sync>;
