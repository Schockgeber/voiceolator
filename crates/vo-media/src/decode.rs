//! Decoding a media file to a stream of 48 kHz mono samples.

use crate::resample::Resample;
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};

pub const RATE: u32 = 48000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub duration_s: Option<f64>,
    pub has_video: bool,
    pub has_audio: bool,
    /// Short description of the sound track, e.g. "aac, 48000 Hz, stereo".
    pub audio_desc: String,
}

/// A decoded sound track: call `read` until it returns false.
pub trait Source {
    /// Append the next block of 48 kHz mono samples; false at the end of the file.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool>;
}

// ---------- ffmpeg ----------

/// Ask ffmpeg what is in the file (parses `ffmpeg -i` output; ffprobe is not needed).
pub fn probe_ffmpeg(ffmpeg: &Path, input: &Path) -> Result<Probe> {
    let out = crate::hidden_command(ffmpeg).arg("-hide_banner").arg("-nostdin").arg("-i").arg(input).output().context("could not run ffmpeg")?;
    let text = String::from_utf8_lossy(&out.stderr);
    Ok(parse_probe(&text))
}

pub(crate) fn parse_probe(text: &str) -> Probe {
    let mut p = Probe::default();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Duration: ") {
            let d = rest.split(',').next().unwrap_or("");
            let parts: Vec<f64> = d.split(':').filter_map(|x| x.trim().parse().ok()).collect();
            if parts.len() == 3 {
                p.duration_s = Some(parts[0] * 3600.0 + parts[1] * 60.0 + parts[2]);
            }
        }
        if l.starts_with("Stream #") {
            if l.contains(": Video: ") && !l.contains("(attached pic)") {
                p.has_video = true;
            }
            if let Some(i) = l.find(": Audio: ") {
                if !p.has_audio {
                    p.audio_desc = l[i + 9..].split(',').take(3).collect::<Vec<_>>().join(",").trim().to_string();
                }
                p.has_audio = true;
            }
        }
    }
    p
}

fn stderr_collector(child: &mut Child) -> std::thread::JoinHandle<String> {
    let mut err = child.stderr.take();
    std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        // keep the end, where ffmpeg puts the actual error
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(4)..].join(" / ")
    })
}

pub struct FfmpegSource {
    child: Child,
    stdout: std::process::ChildStdout,
    stderr: Option<std::thread::JoinHandle<String>>,
    carry: Vec<u8>,
    buf: Vec<u8>,
}

impl FfmpegSource {
    pub fn open(ffmpeg: &Path, input: &Path) -> Result<Self> {
        let mut child = crate::hidden_command(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
            .arg(input)
            // first sound track; pad/realign so sample 0 is the start of the file
            // (keeps the voice in sync with the picture when the sound starts late)
            .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-af", "aresample=async=1:first_pts=0", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("could not start ffmpeg")?;
        let stdout = child.stdout.take().context("ffmpeg stdout")?;
        let stderr = Some(stderr_collector(&mut child));
        Ok(Self { child, stdout, stderr, carry: Vec::new(), buf: vec![0u8; 1 << 16] })
    }
}

impl Source for FfmpegSource {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        let n = self.stdout.read(&mut self.buf)?;
        if n == 0 {
            let status = self.child.wait()?;
            let err = self.stderr.take().and_then(|h| h.join().ok()).unwrap_or_default();
            if !status.success() {
                bail!("ffmpeg could not read the file: {}", if err.is_empty() { status.to_string() } else { err });
            }
            return Ok(false);
        }
        self.carry.extend_from_slice(&self.buf[..n]);
        let whole = self.carry.len() / 4 * 4;
        out.extend(self.carry[..whole].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        self.carry.drain(..whole);
        Ok(true)
    }
}

impl Drop for FfmpegSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------- built-in (Symphonia) ----------

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_AAC, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct BuiltinSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    resample: Option<Resample>,
    sbuf: Option<SampleBuffer<f32>>,
    mono: Vec<f32>,
    /// Encoder delay still to drop (source-rate frames).
    trim: usize,
    done: bool,
}

impl BuiltinSource {
    pub fn open(input: &Path) -> Result<(Self, Probe)> {
        let f = std::fs::File::open(input).with_context(|| format!("cannot open {}", input.display()))?;
        let mss = MediaSourceStream::new(Box::new(f), Default::default());
        let mut hint = Hint::new();
        if let Some(e) = input.extension().and_then(|e| e.to_str()) {
            hint.with_extension(e);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
            .map_err(|e| unsupported(input, e))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
            .context("this file has no sound track the built-in decoder can read")?;
        let params = track.codec_params.clone();
        let decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| unsupported(input, e))?;
        let rate = params.sample_rate.unwrap_or(RATE);
        let duration_s = params.n_frames.map(|n| n as f64 / rate as f64);
        let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let probe = Probe {
            duration_s,
            has_video: crate::VIDEO_EXTENSIONS.contains(&ext.as_str()),
            has_audio: true,
            audio_desc: format!("{} Hz, {} ch", rate, params.channels.map(|c| c.count()).unwrap_or(0)),
        };
        let resample = if rate != RATE { Some(Resample::new(rate, RATE)?) } else { None };
        // Keep the voice in sync: drop the encoder's start-up delay. With gapless mode
        // Symphonia already removes it when it knows it (MP3 LAME header), but not for
        // AAC in MP4, where it is 1024 frames for practically all encoders. (With ffmpeg,
        // the container's own value is used.)
        let trim = match params.delay {
            Some(_) => 0,
            None if params.codec == CODEC_TYPE_AAC => 1024,
            None => 0,
        };
        Ok((Self { track: track.id, format, decoder, resample, sbuf: None, mono: Vec::new(), trim, done: false }, probe))
    }
}

fn unsupported(input: &Path, e: SymErr) -> anyhow::Error {
    anyhow::anyhow!(
        "the built-in decoder can't read {} ({e}). Install ffmpeg (or let Voiceolator download it) to open this format.",
        input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    )
}

impl Source for BuiltinSource {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool> {
        if self.done {
            return Ok(false);
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    self.done = true;
                    if let Some(r) = self.resample.as_mut() {
                        r.finish(out)?;
                    }
                    return Ok(!out.is_empty());
                }
                Err(SymErr::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow::anyhow!("reading the file failed: {e}")),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymErr::DecodeError(e)) => {
                    log::warn!("skipping a damaged audio packet: {e}");
                    continue;
                }
                Err(SymErr::IoError(_)) => continue,
                Err(e) => return Err(anyhow::anyhow!("decoding failed: {e}")),
            };
            let spec = *decoded.spec();
            let ch = spec.channels.count().max(1);
            let need = decoded.capacity() as u64;
            if self.sbuf.as_ref().map(|b| b.capacity() < decoded.capacity() * ch).unwrap_or(true) {
                self.sbuf = Some(SampleBuffer::new(need, spec));
            }
            let sb = self.sbuf.as_mut().unwrap();
            sb.copy_interleaved_ref(decoded);
            self.mono.clear();
            let inv = 1.0 / ch as f32;
            self.mono.extend(sb.samples().chunks_exact(ch).map(|c| c.iter().sum::<f32>() * inv));
            if self.trim > 0 {
                let k = self.trim.min(self.mono.len());
                self.trim -= k;
                self.mono.drain(..k);
            }
            match self.resample.as_mut() {
                Some(r) => r.push(&self.mono, out)?,
                None => out.extend_from_slice(&self.mono),
            }
            return Ok(true);
        }
    }
}

/// Open a file for decoding: with ffmpeg when given, otherwise with the built-in decoders.
pub fn open(input: &Path, ffmpeg: Option<&Path>) -> Result<(Box<dyn Source>, Probe)> {
    if !input.is_file() {
        bail!("file not found: {}", input.display());
    }
    if let Some(ff) = ffmpeg {
        let p = probe_ffmpeg(ff, input)?;
        if !p.has_audio {
            if p.has_video {
                bail!("this video has no sound track");
            }
            bail!("ffmpeg can't read this file (not an audio or video file?)");
        }
        return Ok((Box::new(FfmpegSource::open(ff, input)?), p));
    }
    let (s, p) = BuiltinSource::open(input)?;
    Ok((Box::new(s), p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_info() {
        let t = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'vod.mp4':
  Duration: 01:02:03.50, start: 0.000000, bitrate: 6000 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p, 1920x1080, 6000 kb/s, 60 fps
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 160 kb/s (default)
At least one output file must be specified";
        let p = parse_probe(t);
        assert!(p.has_video && p.has_audio);
        assert!((p.duration_s.unwrap() - 3723.5).abs() < 1e-6);
        assert_eq!(p.audio_desc, "aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo");
        let mp3 = "  Duration: 00:00:10.00, start: 0.025057, bitrate: 128 kb/s
  Stream #0:0: Audio: mp3 (mp3float), 44100 Hz, stereo, fltp, 128 kb/s
  Stream #0:1: Video: mjpeg (Baseline), yuvj420p(pc), 500x500, 90k tbr, 90k tbn (attached pic)";
        let p = parse_probe(mp3);
        assert!(!p.has_video && p.has_audio);
    }
}
