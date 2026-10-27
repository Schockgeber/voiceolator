//! Voiceolator file processing: isolate the voice in an existing audio or video file.
//!
//! The same real-time pipeline the live mode uses (`vo_dsp::Pipeline`) runs over a
//! decoded file, hop by hop, and the result is time-aligned with the original:
//!
//! * Audio is decoded with ffmpeg when it is available (any format), otherwise with the
//!   built-in Symphonia decoders (WAV, MP3, AAC/M4A/MP4, FLAC, OGG/Vorbis, MKV/WebM with
//!   AAC/Vorbis/FLAC, ALAC).
//! * The clean voice is written as WAV (built-in) or MP3/M4A/FLAC (ffmpeg).
//! * For video input, ffmpeg copies the picture unchanged and replaces the sound with the
//!   clean voice (optionally keeping the original sound as a second audio track).
//!
//! ffmpeg is never required for WAV output and is only downloaded after the user agrees
//! (see [`ffmpeg::download`]); everything else runs locally with no network access.

pub mod decode;
pub mod ffmpeg;
pub mod job;
mod parallel;
mod resample;

pub use parallel::default_threads;

pub use job::{process, AudioFormat, Cancelled, Done, Options, Progress, Stage};

use std::path::PathBuf;

/// Per-user data folder for downloaded helpers (ffmpeg):
/// `%LOCALAPPDATA%\Voiceolator` on Windows, `~/.local/share/voiceolator` on Linux.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("Voiceolator"));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .map(|p| p.join("voiceolator"));
    base.unwrap_or_else(|| PathBuf::from("."))
}

/// File extensions the file picker offers (ffmpeg can open more).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "flv", "ts", "mts", "wmv", "mpg", "mpeg", "3gp", "wav", "mp3", "m4a", "aac", "flac",
    "ogg", "oga", "opus", "wma", "aiff", "aif", "caf", "alac",
];

/// Extensions treated as video when ffmpeg is not available to tell.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "flv", "ts", "mts", "wmv", "mpg", "mpeg", "3gp"];

pub(crate) fn hidden_command(program: &std::path::Path) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window flashes
    }
    c
}
