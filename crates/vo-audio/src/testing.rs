//! Simulated real-time streams for testing the host without audio hardware.
//! They pace themselves with the wall clock exactly like a device would, and can
//! simulate clock skew and disconnects.

use crate::backend::{InputStream, OutputStream};
use anyhow::{bail, Result};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct SimInput {
    pub signal: Arc<Vec<f32>>,
    pub pos: Arc<AtomicUsize>,
    pub period: usize,
    pub rate_scale: f64,
    pub fail_at: Option<usize>,
    start: Instant,
    emitted: usize,
    base: usize,
}

impl SimInput {
    pub fn new(signal: Arc<Vec<f32>>, pos: Arc<AtomicUsize>, rate_scale: f64, fail_at: Option<usize>) -> Self {
        let base = pos.load(Ordering::Relaxed);
        Self { signal, pos, period: 480, rate_scale, fail_at, start: Instant::now(), emitted: 0, base }
    }
}

impl InputStream for SimInput {
    fn sample_rate(&self) -> u32 {
        48000
    }
    fn channels(&self) -> usize {
        2
    }
    fn latency_frames(&self) -> usize {
        self.period
    }
    fn read(&mut self, out: &mut Vec<f32>, timeout_ms: u32) -> Result<usize> {
        let due = |emitted: usize| self.start + Duration::from_secs_f64(emitted as f64 / (48000.0 * self.rate_scale));
        let next = due(self.emitted + self.period);
        let now = Instant::now();
        if next > now {
            let wait = next - now;
            if wait > Duration::from_millis(timeout_ms as u64) {
                std::thread::sleep(Duration::from_millis(timeout_ms as u64));
                return Ok(0);
            }
            std::thread::sleep(wait);
        }
        let p = self.base + self.emitted;
        if let Some(f) = self.fail_at {
            if p >= f {
                self.fail_at = None;
                bail!("simulated device removal");
            }
        }
        for i in 0..self.period {
            let s = self.signal.get(p + i).copied().unwrap_or(0.0);
            out.push(s);
            out.push(s);
        }
        self.emitted += self.period;
        self.pos.store(self.base + self.emitted, Ordering::Relaxed);
        Ok(self.period)
    }
}

pub struct SimOutput {
    pub sink: Arc<Mutex<Vec<f32>>>,
    pub period: usize,
    pub rate_scale: f64,
    start: Instant,
    consumed: usize,
    buffered: usize,
    buf_size: usize,
}

impl SimOutput {
    pub fn new(sink: Arc<Mutex<Vec<f32>>>, rate_scale: f64) -> Self {
        Self { sink, period: 480, rate_scale, start: Instant::now(), consumed: 0, buffered: 0, buf_size: 960 }
    }
    fn played_now(&self) -> usize {
        (self.start.elapsed().as_secs_f64() * 48000.0 * self.rate_scale) as usize
    }
}

impl OutputStream for SimOutput {
    fn sample_rate(&self) -> u32 {
        48000
    }
    fn channels(&self) -> usize {
        2
    }
    fn wait(&mut self, _timeout_ms: u32) -> Result<()> {
        // wake every period
        let next = self.start + Duration::from_secs_f64((self.consumed + self.period) as f64 / (48000.0 * self.rate_scale));
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
        Ok(())
    }
    fn writable_frames(&mut self) -> Result<usize> {
        let played = self.played_now();
        let newly = played.saturating_sub(self.consumed).min(self.buffered);
        self.consumed += played.saturating_sub(self.consumed);
        self.buffered -= newly;
        Ok(self.buf_size - self.buffered)
    }
    fn queued_frames(&mut self) -> Result<usize> {
        Ok(self.buffered)
    }
    fn write(&mut self, data: &[f32]) -> Result<()> {
        let frames = data.len() / 2;
        self.buffered += frames;
        let mut s = self.sink.lock();
        s.extend(data.chunks_exact(2).map(|f| f[0]));
        Ok(())
    }
}
