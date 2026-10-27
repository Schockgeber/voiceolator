//! Streaming mono resampler to 48 kHz (FFT based, from rubato), delay-compensated so the
//! output is time-aligned with the input and has exactly `round(n_in * ratio)` samples.

use anyhow::Result;
use rubato::{FftFixedIn, Resampler};

pub struct Resample {
    r: FftFixedIn<f32>,
    inbuf: Vec<f32>,
    skip: usize,
    in_total: u64,
    out_total: u64,
    rate_in: u32,
    rate_out: u32,
}

impl Resample {
    pub fn new(rate_in: u32, rate_out: u32) -> Result<Self> {
        let r = FftFixedIn::<f32>::new(rate_in as usize, rate_out as usize, 1024, 2, 1)?;
        let skip = r.output_delay();
        Ok(Self { r, inbuf: Vec::new(), skip, in_total: 0, out_total: 0, rate_in, rate_out })
    }

    fn expected(&self) -> u64 {
        (self.in_total * self.rate_out as u64 + self.rate_in as u64 / 2) / self.rate_in as u64
    }

    fn emit(&mut self, mut y: &[f32], out: &mut Vec<f32>, limit: Option<u64>) {
        if self.skip > 0 {
            let s = self.skip.min(y.len());
            self.skip -= s;
            y = &y[s..];
        }
        if let Some(l) = limit {
            let room = l.saturating_sub(self.out_total) as usize;
            y = &y[..y.len().min(room)];
        }
        self.out_total += y.len() as u64;
        out.extend_from_slice(y);
    }

    pub fn push(&mut self, x: &[f32], out: &mut Vec<f32>) -> Result<()> {
        self.in_total += x.len() as u64;
        self.inbuf.extend_from_slice(x);
        let mut pos = 0;
        loop {
            let need = self.r.input_frames_next();
            if self.inbuf.len() - pos < need {
                break;
            }
            let y = self.r.process(&[&self.inbuf[pos..pos + need]], None)?;
            pos += need;
            let y = y.into_iter().next().unwrap_or_default();
            self.emit(&y, out, None);
        }
        self.inbuf.drain(..pos);
        Ok(())
    }

    /// Flush the remaining input and the filter delay.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<()> {
        let want = self.expected();
        let rest = std::mem::take(&mut self.inbuf);
        if !rest.is_empty() {
            let y = self.r.process_partial(Some(&[&rest[..]]), None)?;
            self.emit(&y.into_iter().next().unwrap_or_default(), out, Some(want));
        }
        let mut guard = 0;
        while self.out_total < want && guard < 64 {
            let y = self.r.process_partial(None::<&[&[f32]]>, None)?;
            self.emit(&y.into_iter().next().unwrap_or_default(), out, Some(want));
            guard += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_alignment() {
        // A click at 0.5 s must stay at 0.5 s after 44.1 -> 48 kHz.
        let n = 44100;
        let mut x = vec![0.0f32; n];
        x[22050] = 1.0;
        let mut r = Resample::new(44100, 48000).unwrap();
        let mut y = Vec::new();
        for c in x.chunks(777) {
            r.push(c, &mut y).unwrap();
        }
        r.finish(&mut y).unwrap();
        assert_eq!(y.len(), 48000);
        let peak = y.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert!((peak as i64 - 24000).abs() <= 2, "peak at {peak}");
    }
}
