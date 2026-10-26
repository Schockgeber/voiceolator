//! WASAPI backend (Windows 10 2004+ / Windows 11).
pub mod discover;
pub mod duck;
pub mod latency;
pub mod mute;
pub mod streams;

pub use discover::{find_app, find_device, list_audio_apps, list_input_devices, list_output_devices, process_name, AudioApp, DeviceInfo};
pub use duck::{ducked_input_factory, DuckShared, DuckTarget};
pub use mute::unmute_session_ids;
pub use streams::{input_factory, output_factory};
