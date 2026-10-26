//! vo-eval — run the Voiceolator pipeline offline, hop by hop, exactly as in real time.
//!
//! Usage:
//!   vo-eval <in.wav> <out.wav> [--preset balanced|voice|max] [--raw] [--bypass]
//!           [--clarity X] [--suppression X] [--preservation X] [--stats stats.csv]
//!           [--engine dfn3|dpdfnet] [--model dpdfnet.onnx] [--ort path/to/onnxruntime lib]
//!
//! Input must be 48 kHz (any channel count; downmixed to mono). Output is 48 kHz mono
//! float WAV, time-aligned with the input (engine latency removed) for A/B comparison.
//! Prints the real-time factor (processing time / audio duration).

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::time::Instant;
use vo_dsp::{create_engine, process_offline, EngineKind, Pipeline, Preset, Settings};

fn read_mono(path: &str) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(path).with_context(|| format!("open {path}"))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>()?
        }
    };
    let mono = samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    Ok((mono, spec.sample_rate))
}

fn write_mono(path: &str, x: &[f32], sr: u32) -> Result<()> {
    let spec = hound::WavSpec { channels: 1, sample_rate: sr, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in x {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

/// Engine-only path: no attenuation limit, gate, EQ or leveler; default engine parameters.
fn process_raw(kind: &EngineKind, input: &[f32]) -> Result<Vec<f32>> {
    let mut e = create_engine(kind, &Settings::default())?;
    e.set_params(&vo_dsp::EngineParams::default());
    let hop = e.hop_size();
    let lat = e.latency_samples();
    let n = (input.len() + lat).div_ceil(hop);
    let mut x = input.to_vec();
    x.resize(n * hop, 0.0);
    let mut y = vec![0.0; n * hop];
    for h in 0..n {
        e.process(&x[h * hop..(h + 1) * hop], &mut y[h * hop..(h + 1) * hop])?;
    }
    Ok(y[lat..lat + input.len()].to_vec())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        bail!("usage: vo-eval <in.wav> <out.wav> [--preset P] [--raw] [--bypass] [--clarity X] [--suppression X] [--preservation X] [--stats file.csv]");
    }
    let (inp, outp) = (&args[0], &args[1]);
    let mut settings = Settings::default();
    let mut raw = false;
    let mut engine = "dfn3".to_string();
    let mut model = String::from("models/dpdfnet2_48khz_hr.onnx");
    let mut ort_lib = String::from(if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so" });
    let mut stats_path: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        let next = |i: usize| args.get(i + 1).cloned().context("missing value");
        match args[i].as_str() {
            "--preset" => {
                let p = Preset::parse(&next(i)?).context("unknown preset")?;
                settings = Settings::from_preset(p);
                i += 1;
            }
            "--raw" => raw = true,
            "--bypass" => settings.enabled = false,
            "--no-leveler" => settings.leveler = false,
            "--no-tone" => settings.tone_shaping = false,
            "--clarity" => { settings.clarity = next(i)?.parse()?; settings.preset = Preset::Custom; i += 1; }
            "--suppression" => { settings.suppression = next(i)?.parse()?; settings.preset = Preset::Custom; i += 1; }
            "--preservation" => { settings.preservation = next(i)?.parse()?; settings.preset = Preset::Custom; i += 1; }
            "--stats" => { stats_path = Some(next(i)?); i += 1; }
            "--engine" => { engine = next(i)?; i += 1; }
            "--model" => { model = next(i)?; i += 1; }
            "--ort" => { ort_lib = next(i)?; i += 1; }
            a => bail!("unknown argument {a}"),
        }
        i += 1;
    }
    let kind = match engine.as_str() {
        "dfn3" => EngineKind::Dfn3,
        "dpdfnet" => EngineKind::Dpdfnet { model: model.into(), onnxruntime: ort_lib.into() },
        e => bail!("unknown engine {e} (dfn3|dpdfnet)"),
    };
    let (x, sr) = read_mono(inp)?;
    if sr != 48000 {
        bail!("{inp}: sample rate {sr} Hz, expected 48000");
    }
    let t0 = Instant::now();
    let y = if raw {
        process_raw(&kind, &x)?
    } else {
        let mut p = Pipeline::new(create_engine(&kind, &settings)?, settings);
        let (y, stats) = process_offline(&mut p, &x)?;
        if let Some(sp) = stats_path {
            let mut f = std::fs::File::create(sp)?;
            writeln!(f, "hop,input_peak_db,output_peak_db,engine_snr_db,speech_activity,leveler_gain_db,limiter_gain_db")?;
            for (h, s) in stats.iter().enumerate() {
                writeln!(f, "{h},{:.2},{:.2},{:.2},{:.3},{:.2},{:.2}", s.input_peak_db, s.output_peak_db, s.engine_snr_db, s.speech_activity, s.leveler_gain_db, s.limiter_gain_db)?;
            }
        }
        y
    };
    let el = t0.elapsed().as_secs_f64();
    let dur = x.len() as f64 / sr as f64;
    write_mono(outp, &y, sr)?;
    println!("{{\"file\":\"{inp}\",\"seconds\":{dur:.2},\"proc_seconds\":{el:.3},\"rtf\":{:.4}}}", el / dur);
    Ok(())
}
