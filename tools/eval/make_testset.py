#!/usr/bin/env python3
"""Build the Voiceolator evaluation set (the 7 scenarios from the project brief).

Every scenario is written as three time-aligned 48 kHz mono WAVs:
    <name>_mix.wav     what the app hears (speech + background)
    <name>_speech.wav  the speech we want to keep (ground truth, may be silent)
    <name>_noise.wav   everything we want removed

Sources (local copies, used only for testing, never redistributed):
    speech : EARS dataset (48 kHz anechoic studio speech, CC BY-NC 4.0)
             https://github.com/facebookresearch/ears_dataset
    music  : MUSDB18 7-second previews (stems: drums/bass/other/vocals), research use
             https://zenodo.org/records/3270814
    sfx    : ESC-50 environmental sounds (CC BY-NC 3.0)  https://github.com/karolpiczak/ESC-50
    alerts : synthetic "stream alert" chimes generated here

Usage: make_testset.py --data /path/to/data --out /path/to/testset [--seed 1]
"""
import argparse
import glob
import os
import subprocess

import numpy as np
import soundfile as sf
from scipy.signal import butter, resample_poly, sosfiltfilt

SR = 48000
HP70 = butter(4, 70, "highpass", fs=SR, output="sos")


def load(path):
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != SR:
        g = np.gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g).astype(np.float32)
    return x


def active_rms(x, frame=960, rel_db=-40.0):
    """RMS over 'active' frames (within 40 dB of the loudest), a light P.56 stand-in."""
    n = len(x) // frame
    if n == 0:
        return float(np.sqrt(np.mean(x**2) + 1e-12))
    e = (x[: n * frame].reshape(n, frame) ** 2).mean(axis=1)
    thr = e.max() * 10 ** (rel_db / 10)
    act = e[e > thr]
    return float(np.sqrt(act.mean() + 1e-12))


def trim_silence(x, frame=480, rel_db=-45.0):
    n = len(x) // frame
    e = (x[: n * frame].reshape(n, frame) ** 2).mean(axis=1)
    on = np.where(e > e.max() * 10 ** (rel_db / 10))[0]
    if len(on) == 0:
        return x
    return x[max(0, on[0] - 2) * frame : min(n, on[-1] + 3) * frame]


def fit(x, n):
    if len(x) >= n:
        return x[:n]
    return np.pad(x, (0, n - len(x)))


def crossfade_concat(clips, fade=SR // 10):
    out = clips[0].copy()
    ramp = np.linspace(0, 1, fade, dtype=np.float32)
    for c in clips[1:]:
        out[-fade:] = out[-fade:] * (1 - ramp) + c[:fade] * ramp
        out = np.concatenate([out, c[fade:]])
    return out


class Sources:
    def __init__(self, data, rng):
        self.rng = rng
        self.data = data
        self.speakers = sorted(glob.glob(os.path.join(data, "ears", "p*")))
        self.musdb = sorted(glob.glob(os.path.join(data, "musdb", "*", "*.stem.mp4")))
        self.esc = sorted(glob.glob(os.path.join(data, "esc50", "audio", "*.wav")))
        self.cache = os.path.join(data, "musdb_wav")
        os.makedirs(self.cache, exist_ok=True)
        assert self.speakers and self.musdb and self.esc, "missing source data"

    def speech(self, speaker, seconds, kind="freeform"):
        """Clean reference speech. Some EARS takes carry sub-60 Hz rumble; we high-pass the
        reference at 70 Hz so that removing rumble is not scored as damaging speech."""
        files = sorted(glob.glob(os.path.join(self.speakers[speaker], f"*{kind}*.wav")))
        self.rng.shuffle(files)
        clips, total = [], 0
        for f in files:
            c = trim_silence(sosfiltfilt(HP70, load(f)).astype(np.float32))
            clips.append(c)
            total += len(c) + SR // 2
            if total >= seconds * SR:
                break
        gap = np.zeros(SR // 2, np.float32)
        x = np.concatenate([np.concatenate([c, gap]) for c in clips])
        x = fit(x, int(seconds * SR))
        return x / (active_rms(x) + 1e-9) * 10 ** (-23 / 20)  # speech at -23 dB active RMS

    def _stem(self, path, idx):
        key = os.path.basename(path).replace(".stem.mp4", "")
        out = os.path.join(self.cache, f"{key}.{idx}.wav")
        if not os.path.exists(out):
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", path, "-map", f"0:{idx}",
                            "-ac", "1", "-ar", str(SR), out], check=True)
        return load(out)

    def music(self, seconds, vocals=False):
        """Music bed made of several 7-s MUSDB previews; instrumental unless vocals=True."""
        clips = []
        tracks = list(self.musdb)
        self.rng.shuffle(tracks)
        for t in tracks:
            if vocals:
                v = self._stem(t, 4)
                if active_rms(v) < 0.01:  # want clips that actually contain singing
                    continue
                c = self._stem(t, 0)
            else:
                c = self._stem(t, 1) + self._stem(t, 2) + self._stem(t, 3)
            if active_rms(c) < 0.01:
                continue
            clips.append(c / active_rms(c))
            if sum(len(x) for x in clips) > (seconds + 2) * SR:
                break
        return fit(crossfade_concat(clips), int(seconds * SR))

    def sfx_track(self, seconds, events_per_10s=4, level_db=-14.0):
        """Sudden, loud effects (explosions/fireworks, glass, sirens, alarms...) + alerts."""
        n = int(seconds * SR)
        out = np.zeros(n, np.float32)
        n_ev = max(1, int(seconds / 10 * events_per_10s))
        starts = np.sort(self.rng.uniform(0, seconds - 2.5, n_ev))
        for i, st in enumerate(starts):
            if i % 3 == 2:
                ev = alert_chime(self.rng)
            else:
                ev = trim_silence(load(self.rng.choice(self.esc)))[: int(2.5 * SR)]
            ev = ev / (np.abs(ev).max() + 1e-9) * 10 ** (level_db / 20) * 2.5
            s0 = int(st * SR)
            e0 = min(n, s0 + len(ev))
            out[s0:e0] += ev[: e0 - s0]
        return out


def alert_chime(rng):
    """Twitch-style follower/sub alert: bright arpeggio with decay."""
    t = np.arange(int(1.2 * SR)) / SR
    base = rng.choice([523.25, 659.25, 783.99])
    y = np.zeros_like(t)
    for k, mult in enumerate([1.0, 1.25, 1.5, 2.0]):
        tt = t - 0.12 * k
        m = tt > 0
        env = np.exp(-tt[m] * 4.0)
        y[m] += env * (np.sin(2 * np.pi * base * mult * tt[m]) + 0.3 * np.sin(4 * np.pi * base * mult * tt[m]))
    return (y / np.abs(y).max()).astype(np.float32)


def scale_to(noise, speech, snr_db):
    """Scale noise so that active-speech RMS / noise RMS = snr_db."""
    return noise / (active_rms(noise) + 1e-9) * active_rms(speech) * 10 ** (-snr_db / 20)


def write(out, name, speech, noise):
    mix = speech + noise
    peak = np.abs(mix).max()
    if peak > 0.98:  # keep headroom like a real stream
        g = 0.98 / peak
        speech, noise, mix = speech * g, noise * g, mix * g
    for tag, x in (("mix", mix), ("speech", speech), ("noise", noise)):
        sf.write(os.path.join(out, f"{name}_{tag}.wav"), x.astype(np.float32), SR, subtype="FLOAT")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--seconds", type=float, default=30.0)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    rng = np.random.default_rng(a.seed)
    src = Sources(a.data, rng)
    T = a.seconds
    n = int(T * SR)
    Z = np.zeros(n, np.float32)

    # 1. Clear speech, no background.
    s = src.speech(0, T)
    write(a.out, "s1_clean_speech", s, Z)

    # 2. Speech over continuous background music (instrumental at -8 dB, and with singing).
    s = src.speech(1, T)
    write(a.out, "s2a_speech_music", s, scale_to(src.music(T), s, 8))
    s = src.speech(2, T)
    write(a.out, "s2b_speech_music_vocals", s, scale_to(src.music(T, vocals=True), s, 8))

    # 3. Speech over loud game sounds and sudden effects (+ quieter music bed).
    s = src.speech(0, T)
    sfx = src.sfx_track(T, events_per_10s=5)
    sfx = scale_to(sfx, s, 0)  # effect bursts as loud as the voice on average
    write(a.out, "s3_speech_sfx", s, sfx + scale_to(src.music(T), s, 15))

    # 4. Quiet speech over loud music (music 5 dB LOUDER than speech).
    s = src.speech(1, T, kind="regular")
    write(a.out, "s4_quiet_speech_loud_music", s, scale_to(src.music(T), s, -5))

    # 5. Multiple overlapping voices (two speakers talking over each other) + light music.
    s0, s1 = src.speech(0, T), src.speech(2, T)
    s1 = np.roll(s1, SR)  # offset so they partially overlap
    sp = s0 + s1 * 0.8
    write(a.out, "s5_overlapping_voices", sp, scale_to(src.music(T), sp, 12))

    # 6. Music and effects without any speech.
    ref = src.speech(0, T)  # only for level reference
    bg = scale_to(src.music(T, vocals=True), ref, 3) + scale_to(src.sfx_track(T), ref, 3)
    write(a.out, "s6_music_sfx_no_speech", Z, bg)

    # 7. Rapid changes: silence / speech / loud bursts / speech+music, 1.5-3 s blocks.
    s_full = src.speech(1, T)
    music = scale_to(src.music(T), s_full, 3)
    sfx = scale_to(src.sfx_track(T, events_per_10s=10, level_db=-6), s_full, -3)
    speech = np.zeros(n, np.float32)
    noise = np.zeros(n, np.float32)
    t = 0
    pattern = ["silence", "speech", "burst", "speech+music", "music", "speech", "silence", "burst+speech"]
    k = 0
    while t < n:
        L = int(rng.uniform(1.5, 3.0) * SR)
        seg = slice(t, min(n, t + L))
        kind = pattern[k % len(pattern)]
        if "speech" in kind:
            speech[seg] = s_full[seg]
        if "music" in kind:
            noise[seg] += music[seg]
        if "burst" in kind:
            noise[seg] += sfx[seg] + music[seg] * 0.5
        t += L
        k += 1
    write(a.out, "s7_rapid_transitions", speech, noise)
    print("wrote test set to", a.out)


if __name__ == "__main__":
    main()
