//! Clock-drift compensation.
//!
//! The capture side (the device the browser plays to) and the output side (your
//! headphones) run on different hardware clocks. Even a 0.01 % difference adds or
//! removes ~0.4 s per hour, so the buffer between them would slowly overflow or run dry.
//! [`DriftResampler`] keeps the buffer at a target fill by resampling by a tiny,
//! continuously adjusted ratio (inaudible: at most ±0.5 %), using cubic Hermite
//! interpolation.

pub struct DriftResampler {
    /// Input samples consumed per output sample.
    ratio: f64,
    /// Fractional read position between hist[1] and hist[2].
    frac: f64,
    hist: [f32; 4],
    primed: bool,
    target_fill: f64,
    smoothed_fill: f64,
    integ: f64,
    sr: f64,
    max_dev: f64,
}

impl DriftResampler {
    /// `target_fill`: desired buffered samples between producer and this consumer.
    pub fn new(sr: usize, target_fill: usize) -> Self {
        Self {
            ratio: 1.0,
            frac: 0.0,
            hist: [0.0; 4],
            primed: false,
            target_fill: target_fill as f64,
            smoothed_fill: target_fill as f64,
            integ: 0.0,
            sr: sr as f64,
            max_dev: 0.005,
        }
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }
    pub fn target_fill(&self) -> usize {
        self.target_fill as usize
    }
    pub fn set_target_fill(&mut self, t: usize) {
        self.target_fill = t as f64;
    }

    /// Update the resampling ratio from the current buffer fill. Call once per output
    /// block with the number of samples currently queued.
    pub fn update(&mut self, fill: usize, block_len: usize) {
        let dt = block_len as f64 / self.sr;
        // ~0.2 s smoothing of the fill measurement (it jitters by a block).
        let a = (-dt / 0.2).exp();
        self.smoothed_fill = fill as f64 + (self.smoothed_fill - fill as f64) * a;
        let err_s = (self.smoothed_fill - self.target_fill) / self.sr; // seconds
        self.integ = (self.integ + err_s * dt).clamp(-0.5, 0.5);
        let kp = 0.5; // 10 ms error -> 0.5 % speed change (time constant ~2 s)
        let ki = 0.1;
        self.ratio = 1.0 + (kp * err_s + ki * self.integ).clamp(-self.max_dev, self.max_dev);
    }

    /// Fill `out`, pulling input samples from `pull` (returns None when the buffer is
    /// empty). Returns the number of output samples that had to be zero-filled.
    pub fn produce(&mut self, out: &mut [f32], mut pull: impl FnMut() -> Option<f32>) -> usize {
        let mut starved = 0;
        if !self.primed {
            for k in 0..4 {
                match pull() {
                    Some(x) => self.hist[k] = x,
                    None => {
                        out.fill(0.0);
                        return out.len();
                    }
                }
            }
            self.primed = true;
        }
        for o in out.iter_mut() {
            let t = self.frac as f32;
            let [y0, y1, y2, y3] = self.hist;
            // Catmull-Rom / cubic Hermite between y1 and y2.
            let c0 = y1;
            let c1 = 0.5 * (y2 - y0);
            let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
            *o = ((c3 * t + c2) * t + c1) * t + c0;

            self.frac += self.ratio;
            while self.frac >= 1.0 {
                match pull() {
                    Some(x) => {
                        self.hist = [self.hist[1], self.hist[2], self.hist[3], x];
                        self.frac -= 1.0;
                    }
                    None => {
                        // Buffer ran dry: hold position, emit silence for the rest.
                        self.frac = self.frac.min(1.0 - 1e-9);
                        starved += 1;
                        *o = 0.0;
                        break;
                    }
                }
            }
        }
        starved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Simulate a producer that is 0.1 % faster than the consumer for 60 s and check that
    /// the buffer stays near target instead of growing by 60 ms.
    #[test]
    fn absorbs_clock_drift() {
        let sr = 48000;
        let block = 480;
        let target = 2400;
        let mut q: VecDeque<f32> = (0..target).map(|_| 0.0).collect();
        let mut r = DriftResampler::new(sr, target);
        let mut out = vec![0.0; block];
        let mut acc = 0.0f64;
        let mut phase = 0.0f64;
        let mut max_err = 0i64;
        let (mut sum_err, mut n_err) = (0i64, 0i64);
        for step in 0..(60 * sr / block) {
            acc += block as f64 * 1.001;
            while acc >= 1.0 {
                q.push_back((phase).sin() as f32);
                phase += 0.01;
                acc -= 1.0;
            }
            r.update(q.len(), block);
            let fill_before = q.len();
            let starved = r.produce(&mut out, || q.pop_front());
            assert_eq!(starved, 0);
            if step > 20 * sr / block {
                let e = fill_before as i64 - target as i64;
                max_err = max_err.max(e.abs());
                sum_err += e;
                n_err += 1;
            }
        }
        // Fill jitters by up to one block because producer and consumer alternate;
        // the *average* must sit on target, and it must never wander far.
        let mean = sum_err / n_err;
        assert!(mean.abs() < 100, "mean fill error {mean} samples");
        assert!(max_err < 2 * block as i64, "fill drifted by {max_err} samples");
    }

    #[test]
    fn unity_ratio_is_transparent() {
        let mut r = DriftResampler::new(48000, 0);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let mut it = input.iter().copied();
        let mut out = vec![0.0; 50];
        r.produce(&mut out, || it.next());
        // Output lags by one sample (we interpolate between hist[1] and hist[2]).
        for (i, o) in out.iter().enumerate() {
            assert!((o - (i as f32 + 1.0)).abs() < 1e-4, "{i}: {o}");
        }
    }
}
