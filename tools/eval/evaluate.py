#!/usr/bin/env python3
"""Run the real Voiceolator pipeline (vo-eval binary) over the test set and score it.

Metrics per scenario and configuration
  si_sdr / d_si_sdr   scale-invariant SDR of output vs clean speech (and gain over input)
  stoi                short-time objective intelligibility (0..1), 16 kHz
  pesq                PESQ wide-band (1..4.5), 16 kHz
  gap_atten_db        how much quieter the background is in pauses (no speech), out vs in
  speech_resid_db     residual (everything that is not the clean speech) during speech,
                      relative to the background that was there in the input (lower=better)
  dropout_pct         % of speech frames whose energy fell > 10 dB below the clean speech
                      (cut words/syllables)
  dnsmos_sig/bak/ovr  Microsoft DNSMOS P.835 (non-intrusive): speech quality, background
                      intrusiveness, overall (1..5)
  total_atten_db      (no-speech scenario only) output level vs input level
  rtf                 processing time / audio duration (single thread)

Usage: evaluate.py --testset DIR --bin path/to/vo-eval --dnsmos DIR --out DIR
"""
import argparse
import glob
import json
import os
import subprocess

import numpy as np
import onnxruntime as ort
import soundfile as sf
from pesq import pesq
from pystoi import stoi
from scipy.signal import resample_poly

SR = 48000
FRAME = 960  # 20 ms

DP = ["--engine", "dpdfnet"]
NT = ["--no-tone"]  # presets scored without the (linear) clarity EQ / leveler stage
CONFIGS = {
    "input (bypass)": ["--bypass", *NT],
    "DFN3 raw": ["--raw"],
    "DFN3 Balanced": ["--preset", "balanced", *NT],
    "DPDFNet raw": ["--raw", *DP],
    "DPDFNet Balanced": ["--preset", "balanced", *DP, *NT],
    "DPDFNet Voice Priority": ["--preset", "voice", *DP, *NT],
    "DPDFNet Max Suppression": ["--preset", "max", *DP, *NT],
    "DPDFNet Balanced + tone": ["--preset", "balanced", *DP],
}


def rd(p):
    x, sr = sf.read(p, dtype="float32")
    assert sr == SR
    return x


def to16(x):
    return resample_poly(x, 1, 3).astype(np.float32)


def si_sdr(est, ref):
    ref = ref - ref.mean()
    est = est - est.mean()
    a = np.dot(est, ref) / (np.dot(ref, ref) + 1e-12)
    t = a * ref
    return 10 * np.log10(np.sum(t**2) / (np.sum((est - t) ** 2) + 1e-12) + 1e-12)


def frames_energy(x):
    n = len(x) // FRAME
    return (x[: n * FRAME].reshape(n, FRAME) ** 2).mean(axis=1)


class DNSMOS:
    def __init__(self, d):
        self.s = ort.InferenceSession(os.path.join(d, "sig_bak_ovr.onnx"), providers=["CPUExecutionProvider"])
        self.p_ovr = np.poly1d([-0.06766283, 1.11546468, 0.04602535])
        self.p_sig = np.poly1d([-0.08397278, 1.22083953, 0.0052439])
        self.p_bak = np.poly1d([-0.13166888, 1.60915514, -0.39604546])

    def __call__(self, x48):
        x = to16(x48)
        L = int(9.01 * 16000)
        if len(x) < L:
            x = np.pad(x, (0, L - len(x)))
        res = []
        for st in range(0, len(x) - L + 1, 16000):
            seg = x[st : st + L][None, :].astype(np.float32)
            sig, bak, ovr = self.s.run(None, {"input_1": seg})[0][0]
            res.append((self.p_sig(sig), self.p_bak(bak), self.p_ovr(ovr)))
        return np.mean(res, axis=0)


def score(out, mix, speech, noise, dns):
    r = {}
    has_speech = np.abs(speech).max() > 1e-4
    e_s, e_mix, e_out, e_noise = map(frames_energy, (speech, mix, out, noise))
    if has_speech:
        act = active_level = None
        s_act = e_s.max() * 1e-4  # within 40 dB of the loudest speech frame
        speech_frames = e_s > s_act
        gap_frames = (e_s < s_act * 1e-2) & (e_noise > 1e-7)
        r["si_sdr"] = si_sdr(out, speech)
        r["d_si_sdr"] = r["si_sdr"] - si_sdr(mix, speech)
        r["stoi"] = stoi(to16(speech), to16(out), 16000, extended=False)
        try:
            r["pesq"] = pesq(16000, to16(speech), to16(out), "wb")
        except Exception:
            r["pesq"] = float("nan")
        if gap_frames.sum() > 5:
            r["gap_atten_db"] = 10 * np.log10(e_out[gap_frames].sum() / (e_mix[gap_frames].sum() + 1e-12) + 1e-12)
        a = np.dot(out, speech) / (np.dot(speech, speech) + 1e-12)
        resid = frames_energy(out - a * speech)
        if e_noise[speech_frames].sum() > 1e-9:
            r["speech_resid_db"] = 10 * np.log10(resid[speech_frames].sum() / (e_noise[speech_frames].sum() + 1e-12) + 1e-12)
        loud = e_s > e_s.max() * 1e-2  # frames within 20 dB of peak: clearly voiced
        drop = e_out[loud] < e_s[loud] * 0.1
        r["dropout_pct"] = 100.0 * drop.mean()
    else:
        r["total_atten_db"] = 10 * np.log10(e_out.sum() / (e_mix.sum() + 1e-12) + 1e-12)
    sig, bak, ovr = dns(out)
    r["dnsmos_sig"], r["dnsmos_bak"], r["dnsmos_ovr"] = float(sig), float(bak), float(ovr)
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--testset", required=True)
    ap.add_argument("--bin", required=True)
    ap.add_argument("--dnsmos", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--model", default="models/dpdfnet2_48khz_hr.onnx")
    ap.add_argument("--ort", required=True, help="path to the onnxruntime shared library")
    ap.add_argument("--only-configs", default="", help="comma-separated config names to run")
    ap.add_argument("--only-scenarios", default="", help="comma-separated scenario prefixes, e.g. s4,s6")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    dns = DNSMOS(a.dnsmos)
    results = {}
    for mixp in sorted(glob.glob(os.path.join(a.testset, "*_mix.wav"))):
        name = os.path.basename(mixp)[: -len("_mix.wav")]
        if a.only_scenarios and not any(name.startswith(p) for p in a.only_scenarios.split(",")):
            continue
        mix = rd(mixp)
        speech = rd(mixp.replace("_mix", "_speech"))
        noise = rd(mixp.replace("_mix", "_noise"))
        results[name] = {}
        for cname, cargs in CONFIGS.items():
            if a.only_configs and cname not in a.only_configs.split(","):
                continue
            tag = cname.replace(" ", "_").replace("(", "").replace(")", "").lower()
            outp = os.path.join(a.out, f"{name}__{tag}.wav")
            extra = ["--model", a.model, "--ort", a.ort] if "dpdfnet" in cargs else []
            res = subprocess.run([a.bin, mixp, outp, *cargs, *extra], capture_output=True, text=True, check=True)
            perf = json.loads(res.stdout.strip().splitlines()[-1])
            out = rd(outp)
            r = score(out, mix, speech, noise, dns)
            r["rtf"] = perf["rtf"]
            results[name][cname] = {k: float(v) for k, v in r.items()}
            print(name, cname, {k: round(v, 2) for k, v in r.items()}, flush=True)
    with open(os.path.join(a.out, "results.json"), "w") as f:
        json.dump(results, f, indent=1)
    write_markdown(results, os.path.join(a.out, "results.md"))


def write_markdown(results, path):
    cols = ["si_sdr", "d_si_sdr", "stoi", "pesq", "gap_atten_db", "speech_resid_db", "dropout_pct",
            "dnsmos_sig", "dnsmos_bak", "dnsmos_ovr", "total_atten_db", "rtf"]
    lines = []
    for scen, cfgs in results.items():
        present = [c for c in cols if any(c in r for r in cfgs.values())]
        lines.append(f"\n### {scen}\n")
        lines.append("| config | " + " | ".join(present) + " |")
        lines.append("|---|" + "---|" * len(present))
        for cname, r in cfgs.items():
            vals = [("%.2f" % r[c]) if c in r and np.isfinite(r[c]) else "–" for c in present]
            lines.append(f"| {cname} | " + " | ".join(vals) + " |")
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
