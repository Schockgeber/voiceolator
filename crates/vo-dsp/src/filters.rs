//! Small real-time-safe DSP building blocks (no allocation after construction).

use std::f32::consts::PI;

/// RBJ-cookbook biquad, transposed direct form II.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn identity() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 }
    }

    fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn set_highpass(&mut self, sr: f32, f0: f32, q: f32) {
        let w0 = 2.0 * PI * f0 / sr;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn set_peaking(&mut self, sr: f32, f0: f32, q: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * f0 / sr;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Fixed-length delay line used to keep the dry signal aligned with the model output.
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(delay: usize) -> Self {
        Self { buf: vec![0.0; delay.max(1)], pos: 0 }
    }
    /// Push one sample, return the sample from `delay` samples ago.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.buf[self.pos];
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        y
    }
}

/// One-pole smoothing coefficient for a time constant in ms at a given update rate (Hz).
pub fn smoothing_coef(time_ms: f32, rate_hz: f32) -> f32 {
    if time_ms <= 0.0 {
        return 0.0;
    }
    (-1.0 / (time_ms * 0.001 * rate_hz)).exp()
}

pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn lin_to_db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Sample-accurate peak limiter (no lookahead, fast attack, smooth release) followed by a
/// soft clipper as a last line of defence. Guarantees |y| <= ~1.0.
pub struct Limiter {
    ceiling: f32,
    env: f32,
    release: f32,
}

impl Limiter {
    pub fn new(sr: f32, ceiling_db: f32, release_ms: f32) -> Self {
        Self { ceiling: db_to_lin(ceiling_db), env: 1.0, release: smoothing_coef(release_ms, sr) }
    }
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let a = x.abs();
        let target = if a > self.ceiling { self.ceiling / a } else { 1.0 };
        // Instant attack, exponential release.
        self.env = if target < self.env { target } else { target + (self.env - target) * self.release };
        let y = x * self.env;
        // Soft clip above ceiling (should rarely engage).
        if y.abs() > self.ceiling {
            self.ceiling * (y / self.ceiling).tanh()
        } else {
            y
        }
    }
    /// Current gain reduction in dB (<= 0).
    pub fn gain_db(&self) -> f32 {
        lin_to_db(self.env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highpass_blocks_dc() {
        let mut f = Biquad::identity();
        f.set_highpass(48000.0, 100.0, 0.707);
        let mut y = 0.0;
        for _ in 0..48000 {
            y = f.process(1.0);
        }
        assert!(y.abs() < 1e-3, "dc leaked: {y}");
    }

    #[test]
    fn delay_line_delays() {
        let mut d = DelayLine::new(3);
        let out: Vec<f32> = (1..=6).map(|x| d.tick(x as f32)).collect();
        assert_eq!(out, vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn limiter_caps_peaks() {
        let mut l = Limiter::new(48000.0, -1.0, 50.0);
        let ceiling = db_to_lin(-1.0);
        for i in 0..4800 {
            let x = 4.0 * ((i as f32) * 0.05).sin();
            assert!(l.process(x).abs() <= ceiling + 1e-6);
        }
    }
}
