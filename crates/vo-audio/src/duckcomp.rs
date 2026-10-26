//! Gain compensation for "hide the original sound".
//!
//! To stop you hearing the original stream *without* touching any Windows sound settings,
//! Voiceolator turns the source app's volume in the mixer down to [`DUCK_LEVEL`]
//! (−70 dB, inaudible). Windows applies that volume *before* process-loopback capture
//! (that is why a plain mute also silenced the capture), so the captured audio is 70 dB
//! too quiet and must be multiplied back up. The audio engine works in 32-bit float, so
//! this round trip is lossless.
//!
//! Because that ordering is an observed Windows behaviour rather than a documented
//! guarantee, the compensator verifies it from the signal itself:
//!
//! * While undecided, it passes audio unchanged and watches the level. Ducked
//!   post-volume audio can never exceed `DUCK_LEVEL × 2` (that would be a +6 dBFS
//!   original). If real audio does exceed it, capture is pre-volume: no compensation.
//! * In compensated mode, any 10 ms packet louder than the threshold (e.g. a new browser
//!   audio session that hasn't been turned down yet) is passed at unity gain, so it is
//!   never blasted by +70 dB. If that persists for 2 s, it switches to unity mode.

/// Mixer volume used for the hidden original (−70 dB).
pub const DUCK_LEVEL: f32 = 0.000_316_2;
const LOUD: f32 = DUCK_LEVEL * 2.0;
const SIGNAL: f32 = 1e-7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuckMode {
    /// Not enough audio seen yet to tell how Windows applies the app volume.
    Undecided,
    /// Capture sees the turned-down volume: multiply by 1/DUCK_LEVEL.
    Compensated,
    /// Capture is unaffected by the app volume: unity gain.
    Unity,
}

#[derive(Debug)]
pub struct DuckCompensator {
    mode: DuckMode,
    seen: u32,
    loud: u32,
    loud_run_ms: f32,
    warmup_ms: f32,
}

impl Default for DuckCompensator {
    fn default() -> Self {
        Self { mode: DuckMode::Undecided, seen: 0, loud: 0, loud_run_ms: 0.0, warmup_ms: 300.0 }
    }
}

impl DuckCompensator {
    pub fn mode(&self) -> DuckMode {
        self.mode
    }

    /// Call when (re)opening the capture: the mixer ramps volume changes, so the first
    /// few hundred ms are not used for the decision.
    pub fn warm_up(&mut self) {
        self.warmup_ms = 300.0;
    }

    /// Process one captured packet in place (interleaved; `ms` = its duration).
    /// Returns true if the packet looked un-ducked (caller should re-apply ducking now).
    pub fn process(&mut self, buf: &mut [f32], ms: f32) -> bool {
        let peak = buf.iter().fold(0f32, |m, x| m.max(x.abs()));
        let has_signal = peak > SIGNAL;
        let loud = peak > LOUD;
        match self.mode {
            DuckMode::Undecided => {
                if self.warmup_ms > 0.0 {
                    self.warmup_ms -= ms;
                    return loud;
                }
                if has_signal {
                    self.seen += 1;
                    if loud {
                        self.loud += 1;
                    }
                    if self.loud >= 3 {
                        self.mode = DuckMode::Unity;
                    } else if self.seen >= 8 {
                        self.mode = DuckMode::Compensated;
                    }
                }
                // pass unchanged while undecided (at worst briefly quiet)
                loud
            }
            DuckMode::Compensated => {
                if loud {
                    self.loud_run_ms += ms;
                    if self.loud_run_ms > 2000.0 {
                        log::warn!("captured audio is not affected by the app volume; disabling compensation");
                        self.mode = DuckMode::Unity;
                    }
                    true // unity gain for this packet
                } else {
                    self.loud_run_ms = 0.0;
                    let g = 1.0 / DUCK_LEVEL;
                    for x in buf.iter_mut() {
                        *x *= g;
                    }
                    false
                }
            }
            DuckMode::Unity => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(level: f32) -> Vec<f32> {
        (0..960).map(|i| level * ((i as f32) * 0.07).sin()).collect()
    }

    #[test]
    fn post_volume_capture_is_restored() {
        let mut c = DuckCompensator::default();
        // a loud packet during the volume ramp at start must not cause a wrong decision
        let mut p = packet(0.3);
        c.process(&mut p, 10.0);
        for _ in 0..70 {
            let mut p = packet(0.3 * DUCK_LEVEL);
            c.process(&mut p, 10.0);
        }
        assert_eq!(c.mode(), DuckMode::Compensated);
        let mut p = packet(0.3 * DUCK_LEVEL);
        c.process(&mut p, 10.0);
        let peak = p.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!((peak - 0.3).abs() < 0.01, "peak {peak}");
    }

    #[test]
    fn pre_volume_capture_is_left_alone() {
        let mut c = DuckCompensator::default();
        for _ in 0..40 {
            let mut p = packet(0.2);
            c.process(&mut p, 10.0);
            assert!(p.iter().all(|x| x.abs() <= 0.2001));
        }
        assert_eq!(c.mode(), DuckMode::Unity);
    }

    #[test]
    fn undecked_new_session_is_never_boosted() {
        let mut c = DuckCompensator::default();
        for _ in 0..70 {
            let mut p = packet(0.3 * DUCK_LEVEL);
            c.process(&mut p, 10.0);
        }
        let mut p = packet(0.5); // a new, not yet turned-down session
        assert!(c.process(&mut p, 10.0));
        assert!(p.iter().all(|x| x.abs() <= 0.5001));
        assert_eq!(c.mode(), DuckMode::Compensated);
    }

    #[test]
    fn silence_does_not_decide() {
        let mut c = DuckCompensator::default();
        for _ in 0..100 {
            let mut p = vec![0.0; 960];
            c.process(&mut p, 10.0);
        }
        assert_eq!(c.mode(), DuckMode::Undecided);
    }
}
