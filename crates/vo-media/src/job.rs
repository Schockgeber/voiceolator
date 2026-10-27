//! One file in, one clean-voice file out.

use crate::decode::{self, Probe, RATE};
use anyhow::{bail, Context, Result};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use vo_dsp::{EngineKind, Settings};

/// The user cancelled.
#[derive(Debug, Clone, Copy)]
pub struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}
impl std::error::Error for Cancelled {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AudioFormat {
    #[default]
    Wav,
    Mp3,
    M4a,
    Flac,
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 4] = [AudioFormat::Wav, AudioFormat::Mp3, AudioFormat::M4a, AudioFormat::Flac];
    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Flac => "flac",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Wav => "WAV (lossless, works without ffmpeg)",
            AudioFormat::Mp3 => "MP3 (small)",
            AudioFormat::M4a => "M4A / AAC (small)",
            AudioFormat::Flac => "FLAC (lossless, smaller than WAV)",
        }
    }
    pub fn needs_ffmpeg(self) -> bool {
        self != AudioFormat::Wav
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.ext().eq_ignore_ascii_case(s) || (s.eq_ignore_ascii_case("aac") && *f == AudioFormat::M4a))
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub engine: EngineKind,
    /// Processing settings (preset/sliders); volume, mute and on/off are ignored.
    pub settings: Settings,
    /// None = next to the input file.
    pub out_dir: Option<PathBuf>,
    pub audio_format: AudioFormat,
    /// For video input: write a video with the clean voice (needs ffmpeg).
    pub make_video: bool,
    /// For video output: keep the original sound as a second audio track.
    pub keep_original_track: bool,
    pub ffmpeg: Option<PathBuf>,
    /// Worker threads (CPU cores) to use; 0 = automatic (half the cores, at most 4).
    pub threads: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Opening,
    Processing,
    Finishing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub stage: Stage,
    /// 0..1 when the duration is known.
    pub fraction: Option<f32>,
    /// Audio seconds processed per wall-clock second.
    pub speed: Option<f32>,
    pub seconds_done: f64,
}

#[derive(Clone, Debug)]
pub struct Done {
    pub output: PathBuf,
    pub video: bool,
    pub audio_seconds: f64,
    pub elapsed_s: f64,
    pub engine: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Plan {
    Wav,
    Audio(AudioFormat),
    Video { muxer: &'static str, ext: &'static str },
}

fn video_container(input: &Path) -> (&'static str, &'static str) {
    match input.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("mp4") => ("mp4", "mp4"),
        Some("m4v") => ("mp4", "m4v"),
        Some("mov") => ("mov", "mov"),
        // Everything else goes into Matroska, which accepts any video codec + AAC.
        _ => ("matroska", "mkv"),
    }
}

fn plan(input: &Path, probe: &Probe, o: &Options) -> (Plan, Vec<String>) {
    let mut notes = Vec::new();
    let ff = o.ffmpeg.is_some();
    if probe.has_video && o.make_video {
        if ff {
            let (muxer, ext) = video_container(input);
            return (Plan::Video { muxer, ext }, notes);
        }
        notes.push("ffmpeg is not available, so only the clean voice was saved (as audio, without the picture).".into());
    }
    if o.audio_format.needs_ffmpeg() {
        if ff {
            return (Plan::Audio(o.audio_format), notes);
        }
        notes.push(format!("Saved as WAV: {} needs ffmpeg.", o.audio_format.ext().to_uppercase()));
    }
    (Plan::Wav, notes)
}

/// `name (voice).ext`, or `name (voice 2).ext`, ... if that exists.
pub fn output_path(input: &Path, out_dir: Option<&Path>, ext: &str) -> PathBuf {
    let dir = out_dir.map(Path::to_path_buf).or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem} (voice).{ext}") } else { format!("{stem} (voice {n}).{ext}") };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

fn temp_path(final_path: &Path) -> PathBuf {
    let name = final_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    final_path.with_file_name(format!(".{name}.part"))
}

enum Sink {
    Wav { w: hound::WavWriter<BufWriter<std::fs::File>>, seed: u32 },
    Ffmpeg { child: Child, stdin: Option<ChildStdin>, err: Option<std::thread::JoinHandle<String>>, bytes: Vec<u8> },
}

fn collect_stderr(child: &mut Child) -> Option<std::thread::JoinHandle<String>> {
    let mut e = child.stderr.take()?;
    Some(std::thread::spawn(move || {
        let mut s = String::new();
        let _ = e.read_to_string(&mut s);
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(4)..].join(" / ")
    }))
}

impl Sink {
    fn wav(path: &Path) -> Result<Self> {
        let spec = hound::WavSpec { channels: 1, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let w = hound::WavWriter::create(path, spec).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(Sink::Wav { w, seed: 0x1234_5678 })
    }

    fn ffmpeg(ffmpeg: &Path, input: &Path, plan: &Plan, keep_original: bool, out: &Path) -> Result<Self> {
        let mut c = crate::hidden_command(ffmpeg);
        c.args(["-hide_banner", "-nostdin", "-v", "error", "-y"]);
        let pipe_in = ["-thread_queue_size", "4096", "-f", "f32le", "-ar", "48000", "-ac", "1", "-i", "pipe:0"];
        match plan {
            Plan::Video { muxer, .. } => {
                c.arg("-i").arg(input).args(pipe_in);
                c.args(["-map", "0:V", "-map", "1:a"]);
                // MP4/MOV: AAC (its encoder delay is signalled there). Matroska does not
                // signal AAC's delay (the voice would be ~21 ms late), so use Opus, which
                // it handles exactly, or FLAC if this ffmpeg has no Opus encoder.
                let voice: &[&str] = if *muxer != "matroska" {
                    &["aac", "-b:a:0", "128k"]
                } else if crate::ffmpeg::has_encoder(ffmpeg, "libopus") {
                    &["libopus", "-b:a:0", "128k"]
                } else {
                    &["flac"]
                };
                if keep_original {
                    c.args(["-map", "0:a", "-c:v", "copy", "-c:a", "copy", "-c:a:0"]).args(voice);
                    c.args(["-metadata:s:a:1", "title=Original sound", "-disposition:a:1", "0"]);
                } else {
                    c.args(["-c:v", "copy", "-c:a:0"]).args(voice);
                }
                c.args(["-metadata:s:a:0", "title=Voice only (Voiceolator)", "-disposition:a:0", "default", "-map_metadata", "0", "-map_chapters", "0"]);
                if *muxer == "matroska" {
                    c.args(["-map", "0:s?", "-c:s", "copy"]);
                } else {
                    c.args(["-movflags", "+faststart"]);
                }
                c.args(["-f", muxer]);
            }
            Plan::Audio(f) => {
                c.args(pipe_in);
                match f {
                    AudioFormat::Mp3 => c.args(["-c:a", "libmp3lame", "-q:a", "2", "-f", "mp3"]),
                    AudioFormat::M4a => c.args(["-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart", "-f", "ipod"]),
                    AudioFormat::Flac => c.args(["-c:a", "flac", "-sample_fmt", "s16", "-f", "flac"]),
                    AudioFormat::Wav => c.args(["-c:a", "pcm_s16le", "-f", "wav"]),
                };
            }
            Plan::Wav => unreachable!(),
        }
        c.arg(out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = c.spawn().context("could not start ffmpeg")?;
        let stdin = child.stdin.take();
        let err = collect_stderr(&mut child);
        Ok(Sink::Ffmpeg { child, stdin, err, bytes: Vec::new() })
    }

    fn write(&mut self, x: &[f32]) -> Result<()> {
        match self {
            Sink::Wav { w, seed } => {
                for &v in x {
                    // TPDF dither to 16 bit
                    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let a = (*seed >> 8) as f32 / (1u32 << 24) as f32;
                    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let b = (*seed >> 8) as f32 / (1u32 << 24) as f32;
                    let s = (v * 32767.0 + (a - b)).round().clamp(-32768.0, 32767.0) as i16;
                    w.write_sample(s)?;
                }
                Ok(())
            }
            Sink::Ffmpeg { stdin, bytes, .. } => {
                bytes.clear();
                bytes.extend(x.iter().flat_map(|v| v.to_le_bytes()));
                let Some(s) = stdin.as_mut() else { bail!("ffmpeg stopped") };
                if s.write_all(bytes).is_err() {
                    *stdin = None;
                    bail!("ffmpeg stopped unexpectedly");
                }
                Ok(())
            }
        }
    }

    fn finish(self) -> Result<()> {
        match self {
            Sink::Wav { w, .. } => w.finalize().context("finishing the WAV file failed"),
            Sink::Ffmpeg { mut child, stdin, err, .. } => {
                drop(stdin);
                let status = child.wait()?;
                let msg = err.and_then(|h| h.join().ok()).unwrap_or_default();
                if !status.success() {
                    bail!("{}", ffmpeg_error(&msg));
                }
                Ok(())
            }
        }
    }

    /// Stop and collect ffmpeg's own explanation (after a write failed).
    fn fail(self) -> String {
        match self {
            Sink::Wav { .. } => String::new(),
            Sink::Ffmpeg { mut child, stdin, err, .. } => {
                drop(stdin);
                let _ = child.wait();
                ffmpeg_error(&err.and_then(|h| h.join().ok()).unwrap_or_default())
            }
        }
    }

    fn kill(self) {
        if let Sink::Ffmpeg { mut child, .. } = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn ffmpeg_error(msg: &str) -> String {
    if msg.contains("libmp3lame") || msg.contains("Unknown encoder") {
        return format!("this ffmpeg can't write MP3 ({msg}). Choose M4A, FLAC or WAV.");
    }
    if msg.is_empty() {
        "ffmpeg failed".into()
    } else {
        format!("ffmpeg failed: {msg}")
    }
}

/// Process one file. `progress` is called a few times per second.
pub fn process(input: &Path, o: &Options, progress: &mut dyn FnMut(Progress), cancel: &AtomicBool) -> Result<Done> {
    let t0 = Instant::now();
    progress(Progress { stage: Stage::Opening, fraction: None, speed: None, seconds_done: 0.0 });
    let (mut src, probe) = decode::open(input, o.ffmpeg.as_deref())?;
    let (plan, notes) = plan(input, &probe, o);
    let ext = match &plan {
        Plan::Wav => "wav",
        Plan::Audio(f) => f.ext(),
        Plan::Video { ext, .. } => ext,
    };
    if let Some(d) = &o.out_dir {
        std::fs::create_dir_all(d).with_context(|| format!("cannot create the output folder {}", d.display()))?;
    }
    let out = output_path(input, o.out_dir.as_deref(), ext);
    let tmp = temp_path(&out);
    let mut sink = match &plan {
        Plan::Wav => Sink::wav(&tmp)?,
        p => Sink::ffmpeg(o.ffmpeg.as_deref().unwrap(), input, p, o.keep_original_track, &tmp)?,
    };

    let result = (|| -> Result<(u64, String)> {
        let mut settings = o.settings;
        settings.enabled = true;
        settings.mute = false;
        settings.volume = 1.0;
        let total = probe.duration_s.map(|d| d * RATE as f64);
        let threads = if o.threads == 0 { crate::parallel::default_threads() } else { o.threads };
        let mut last = Instant::now();
        crate::parallel::run(
            &mut *src,
            &o.engine,
            settings,
            threads,
            &mut |x| sink.write(x),
            &mut |written| {
                if last.elapsed().as_millis() >= 200 {
                    last = Instant::now();
                    let done = written as f64 / RATE as f64;
                    progress(Progress {
                        stage: Stage::Processing,
                        fraction: total.map(|t| (written as f64 / t).min(1.0) as f32),
                        speed: Some((done / t0.elapsed().as_secs_f64().max(1e-3)) as f32),
                        seconds_done: done,
                    });
                }
            },
            cancel,
        )
    })();

    let (n, engine) = match result {
        Ok(v) => v,
        Err(e) => {
            let detail = if e.is::<Cancelled>() {
                sink.kill();
                String::new()
            } else {
                sink.fail()
            };
            let _ = std::fs::remove_file(&tmp);
            if !detail.is_empty() && e.to_string().contains("ffmpeg stopped") {
                bail!("{detail}");
            }
            return Err(e);
        }
    };
    progress(Progress { stage: Stage::Finishing, fraction: Some(1.0), speed: None, seconds_done: n as f64 / RATE as f64 });
    if let Err(e) = sink.finish() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &out).with_context(|| format!("cannot create {}", out.display()))?;
    Ok(Done {
        output: out,
        video: matches!(plan, Plan::Video { .. }),
        audio_seconds: n as f64 / RATE as f64,
        elapsed_s: t0.elapsed().as_secs_f64(),
        engine,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_do_not_overwrite() {
        let d = std::env::temp_dir().join(format!("vo-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let input = d.join("vod.mp4");
        let a = output_path(&input, None, "mp4");
        assert_eq!(a.file_name().unwrap(), "vod (voice).mp4");
        std::fs::write(&a, b"x").unwrap();
        assert_eq!(output_path(&input, None, "mp4").file_name().unwrap(), "vod (voice 2).mp4");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn plan_without_ffmpeg_falls_back_to_wav() {
        let o = Options {
            engine: EngineKind::Dfn3,
            settings: Settings::default(),
            out_dir: None,
            audio_format: AudioFormat::Mp3,
            make_video: true,
            keep_original_track: false,
            ffmpeg: None,
            threads: 0,
        };
        let probe = Probe { duration_s: Some(1.0), has_video: true, has_audio: true, audio_desc: String::new() };
        let (p, notes) = plan(Path::new("a.mp4"), &probe, &o);
        assert_eq!(p, Plan::Wav);
        assert_eq!(notes.len(), 2);
        let o2 = Options { ffmpeg: Some("ffmpeg".into()), ..o };
        assert_eq!(plan(Path::new("a.webm"), &probe, &o2).0, Plan::Video { muxer: "matroska", ext: "mkv" });
    }

    /// End-to-end through the built-in decoder: a 44.1 kHz stereo WAV becomes a 48 kHz
    /// mono WAV of the same duration.
    #[test]
    fn wav_roundtrip_builtin() {
        let d = std::env::temp_dir().join(format!("vo-media-rt-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let input = d.join("clip.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 44100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&input, spec).unwrap();
        for i in 0..44100 {
            let v = ((i as f32 * 0.03).sin() * 8000.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let o = Options {
            engine: EngineKind::Dfn3,
            settings: Settings::default(),
            out_dir: None,
            audio_format: AudioFormat::Wav,
            make_video: true,
            keep_original_track: false,
            ffmpeg: None,
            threads: 0,
        };
        let done = process(&input, &o, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        let r = hound::WavReader::open(&done.output).unwrap();
        assert_eq!(r.spec().sample_rate, 48000);
        assert_eq!(r.spec().channels, 1);
        assert_eq!(r.len(), 48000);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
