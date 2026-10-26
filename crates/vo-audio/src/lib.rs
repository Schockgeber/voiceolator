//! Voiceolator audio host and platform backends.
//!
//! * [`backend`]   – stream traits and capture/output selection types
//! * [`duckcomp`]  – gain compensation for hiding the original app sound
//! * [`host`]      – capture → DSP → render threads, recovery, telemetry
//! * [`telemetry`] – lock-free meters/latency numbers for the UI
//! * [`windows`]   – WASAPI implementation (process loopback, device capture, render)
//! * [`linux`]     – PulseAudio/PipeWire implementation (private capture sink, playback)
//! * [`watchdog`]  – helper process that undoes system changes if Voiceolator crashes
//! * [`testing`]   – simulated real-time streams for hardware-free tests

pub mod backend;
pub mod duckcomp;
pub mod host;
pub mod rt;
pub mod telemetry;
pub mod testing;
pub mod watchdog;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;

pub use backend::{CaptureSource, OutputTarget};
pub use host::{AudioHost, HostConfig, HostEvent, Level};
pub use telemetry::Telemetry;
