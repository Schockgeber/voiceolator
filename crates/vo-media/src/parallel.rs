//! Run the voice pipeline over a long file on several CPU cores.
//!
//! The decoded sound is cut into 60 s segments. Each worker thread owns its own pipeline
//! (models are not shared between threads) and processes one segment at a time, starting
//! 4 s early so the model's internal state has settled when the segment begins (that
//! warm-up output is discarded). Neighbouring segments overlap by 10 ms and are
//! cross-faded, so the joins are inaudible. Results are written strictly in order.
//! With one worker the result equals plain sequential processing apart from those joins.

use crate::decode::{Source, RATE};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use vo_dsp::{create_engine, EngineKind, Pipeline, Settings};

// (shorter in unit tests so they run quickly)
const SEG: usize = if cfg!(test) { 5 } else { 60 } * RATE as usize;
const PRE: usize = if cfg!(test) { 1 } else { 4 } * RATE as usize;
const FADE: usize = 480;

struct Seg {
    k: usize,
    input: Vec<f32>,
    discard: usize,
    out_len: usize,
}

enum Msg {
    Ready { lat: usize, name: String },
    Done(usize, Vec<f32>),
    Failed(anyhow::Error),
}

/// Default worker count: half the cores, at most 4 (the game keeps the rest).
pub fn default_threads() -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    (n / 2).clamp(1, 4)
}

fn worker(engine: EngineKind, settings: Settings, rx: Arc<Mutex<mpsc::Receiver<Seg>>>, tx: mpsc::Sender<Msg>, cancel: &AtomicBool, stop: &AtomicBool, processed: &AtomicU64) {
    let mut pipe = match create_engine(&engine, &settings) {
        Ok(e) => Pipeline::new(e, settings),
        Err(e) => {
            let _ = tx.send(Msg::Failed(e));
            return;
        }
    };
    let hop = pipe.hop_size();
    let _ = tx.send(Msg::Ready { lat: pipe.latency_samples(), name: pipe.engine_name().to_string() });
    let mut y = vec![0.0f32; hop];
    loop {
        let seg = match rx.lock().map(|r| r.recv()) {
            Ok(Ok(s)) => s,
            _ => return,
        };
        let need = seg.discard + seg.out_len;
        let mut input = seg.input;
        input.resize(need.div_ceil(hop) * hop, 0.0);
        let mut out = Vec::with_capacity(seg.out_len);
        let mut produced = 0usize;
        for h in input.chunks_exact(hop) {
            if produced >= need {
                break;
            }
            if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
                return;
            }
            if let Err(e) = pipe.process_hop(h, &mut y) {
                let _ = tx.send(Msg::Failed(e));
                return;
            }
            let from = seg.discard.saturating_sub(produced).min(hop);
            out.extend_from_slice(&y[from..]);
            processed.fetch_add((hop - from) as u64, Ordering::Relaxed);
            produced += hop;
        }
        out.truncate(seg.out_len);
        if tx.send(Msg::Done(seg.k, out)).is_err() {
            return;
        }
    }
}

/// Process everything `src` delivers; returns (samples, engine name).
pub fn run(
    src: &mut dyn Source,
    engine: &EngineKind,
    settings: Settings,
    threads: usize,
    write: &mut dyn FnMut(&[f32]) -> Result<()>,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(u64, String)> {
    let threads = threads.max(1);
    // room for every segment that can be in flight, so sending never blocks
    let (seg_tx, seg_rx) = mpsc::sync_channel::<Seg>(threads + 2);
    let seg_rx = Arc::new(Mutex::new(seg_rx));
    let (res_tx, res_rx) = mpsc::channel::<Msg>();
    let stop = AtomicBool::new(false);
    let stop = &stop;
    // samples finished by all workers (for smooth progress within long segments)
    let processed = AtomicU64::new(0);
    let processed = &processed;
    std::thread::scope(|s| {
        for i in 0..threads {
            let (e, rx, tx) = (engine.clone(), seg_rx.clone(), res_tx.clone());
            std::thread::Builder::new().name(format!("vo-file-{i}")).spawn_scoped(s, move || worker(e, settings, rx, tx, cancel, stop, processed))?;
        }
        drop(res_tx);
        let mut report = |written: u64| progress(written.max(processed.load(Ordering::Relaxed)));
        let r = coordinate(src, threads, &seg_tx, &res_rx, write, &mut report, cancel);
        drop(seg_tx); // lets the workers finish
        if r.is_err() {
            stop.store(true, Ordering::Relaxed);
            if cancel.load(Ordering::Relaxed) {
                // workers that stopped because of the cancel look like failures
                return Err(crate::Cancelled.into());
            }
        }
        r
    })
}

fn coordinate(
    src: &mut dyn Source,
    threads: usize,
    seg_tx: &mpsc::SyncSender<Seg>,
    res_rx: &mpsc::Receiver<Msg>,
    write: &mut dyn FnMut(&[f32]) -> Result<()>,
    progress: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(u64, String)> {
    let (lat, name) = loop {
        match res_rx.recv() {
            Ok(Msg::Ready { lat, name, .. }) => break (lat, name),
            Ok(Msg::Failed(e)) => return Err(e),
            Ok(Msg::Done(..)) => unreachable!(),
            Err(_) => bail!("the voice engine could not start"),
        }
    };
    let max_in_flight = threads + 2;
    let mut buf: Vec<f32> = Vec::with_capacity(SEG + PRE);
    let mut buf_start = 0usize;
    let mut chunk = Vec::with_capacity(1 << 16);
    let (mut n_in, mut eof) = (0usize, false);
    let (mut next_k, mut in_flight, mut next_write, mut written) = (0usize, 0usize, 0usize, 0u64);
    let mut last_k: Option<usize> = None;
    let mut pending: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
    let mut tail: Vec<f32> = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!(crate::Cancelled);
        }
        // 1. hand out every segment whose input is complete
        while last_k.is_none() && in_flight < max_in_flight {
            let start = next_k * SEG;
            if !(eof || n_in >= start + SEG + FADE + lat) {
                break;
            }
            if n_in == 0 {
                bail!("the sound track is empty");
            }
            let is_last = eof && n_in <= start + SEG + FADE;
            let out_len = if is_last { n_in - start } else { SEG + FADE };
            let in_start = start.saturating_sub(PRE);
            let in_end = (start + out_len + lat).min(n_in);
            let input = buf[in_start - buf_start..in_end - buf_start].to_vec();
            if seg_tx.send(Seg { k: next_k, input, discard: start - in_start + lat, out_len }).is_err() {
                bail!("the voice engine stopped");
            }
            in_flight += 1;
            if is_last {
                last_k = Some(next_k);
            }
            next_k += 1;
            let keep_from = (next_k * SEG).saturating_sub(PRE);
            if keep_from > buf_start {
                buf.drain(..(keep_from - buf_start).min(buf.len()));
                buf_start = keep_from;
            }
        }
        // 2. collect results; block only when nothing else can move forward
        let need_more_input = last_k.is_none() && !eof && in_flight < max_in_flight;
        let msg = if in_flight > 0 && !need_more_input {
            // with a timeout, so a cancel is noticed even while every worker is busy
            match res_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => bail!("the voice engine stopped"),
            }
        } else {
            res_rx.try_recv().ok()
        };
        match msg {
            Some(Msg::Failed(e)) => return Err(e),
            Some(Msg::Done(k, out)) => {
                in_flight -= 1;
                pending.insert(k, out);
            }
            _ => {}
        }
        while let Some(mut out) = pending.remove(&next_write) {
            let is_last = last_k == Some(next_write);
            if next_write > 0 {
                let n = FADE.min(out.len()).min(tail.len());
                for i in 0..n {
                    let w = (i as f32 + 0.5) / FADE as f32;
                    out[i] = tail[i] * (1.0 - w) + out[i] * w;
                }
            }
            let cut = if is_last { out.len() } else { out.len() - FADE };
            write(&out[..cut])?;
            written += cut as u64;
            tail = out[cut..].to_vec();
            progress(written);
            next_write += 1;
            if is_last {
                return Ok((n_in as u64, name));
            }
        }
        progress(written);
        // 3. decode more (about a second at a time)
        if need_more_input {
            let target = n_in + RATE as usize;
            while n_in < target {
                chunk.clear();
                let more = src.read(&mut chunk)?;
                n_in += chunk.len();
                buf.extend_from_slice(&chunk);
                if !more {
                    eof = true;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Vecs(Vec<f32>, usize);
    impl Source for Vecs {
        fn read(&mut self, out: &mut Vec<f32>) -> Result<bool> {
            let end = (self.1 + 7000).min(self.0.len());
            out.extend_from_slice(&self.0[self.1..end]);
            self.1 = end;
            Ok(self.1 < self.0.len())
        }
    }

    fn signal(n: usize) -> Vec<f32> {
        (0..n).map(|i| 0.2 * ((i as f32) * 0.021).sin() * (((i / 9000) % 2) as f32)).collect()
    }

    /// Several workers must give the same length and (apart from the joins) the same
    /// result as one worker, for lengths around the segment boundaries.
    #[test]
    fn parallel_matches_sequential() {
        for n in [1000, SEG - 100, SEG + FADE / 2, 2 * SEG + 12345] {
            let x = signal(n);
            let run_with = |t: usize| {
                let mut out = Vec::new();
                let (len, _) = run(&mut Vecs(x.clone(), 0), &EngineKind::Dfn3, Settings::default(), t, &mut |s| Ok(out.extend_from_slice(s)), &mut |_| {}, &AtomicBool::new(false)).unwrap();
                assert_eq!(len as usize, n);
                out
            };
            let a = run_with(1);
            let b = run_with(3);
            assert_eq!(a.len(), n);
            assert_eq!(b.len(), n);
            let err: f32 = a.iter().zip(&b).map(|(p, q)| (p - q).abs()).fold(0.0, f32::max);
            // input peak is 0.2; differences only come from the joins and must be small
            assert!(err <= 0.01, "n={n} max diff {err}");
        }
    }
}
