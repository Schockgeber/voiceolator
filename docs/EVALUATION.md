# Evaluation: Stage 4a (offline, real pipeline)

Date: 2026-10-24. All numbers come from running the **exact real-time code path** (`vo-eval`,
hop by hop, 10 ms frames) over a 30-second test file per scenario. The test machine was an
Intel Xeon @ 2.1 GHz cloud vCPU, single thread. A desktop gaming CPU is expected to be
about 2× faster, but this is still to be measured on the user's PC with
`voiceolator-cli bench`.

Full tables: [eval_results_full.md](eval_results_full.md) and [eval_results.json](eval_results.json).
To reproduce:

```
python tools/eval/make_testset.py --data DATA --out TESTSET
python tools/eval/evaluate.py --testset TESTSET --bin target/release/vo-eval \
       --dnsmos DATA/dnsmos --ort path/to/libonnxruntime.so --out OUT
```

## Test set (the 7 scenarios from the brief)

| # | Scenario | Construction (48 kHz) |
|---|---|---|
| 1 | Clear speech | EARS studio speech, conversational ("freeform") |
| 2a | Speech + continuous music | instrumental MUSDB stems, music 8 dB below the voice |
| 2b | Speech + music with singing | full MUSDB mixes containing vocals, 8 dB below the voice |
| 3 | Speech + loud game sounds / sudden effects | ESC-50 explosions, glass, sirens, alarms, footsteps… plus synthetic stream alerts, as loud as the voice, plus a quiet music bed |
| 4 | Quiet speech over loud music | music **5 dB louder** than the voice |
| 5 | Overlapping voices | two speakers talking over each other, plus light music |
| 6 | Music and effects, no speech | music with singing + effects |
| 7 | Rapid changes | 1.5–3 s blocks: silence / speech / bursts / speech+music / music |

The reference speech is high-passed at 70 Hz. Some EARS takes contain sub-60 Hz rumble,
and without the high-pass, removing it was scored as speech damage.

## Metrics

* **SI-SDR**: scale-invariant SDR vs the clean voice, in dB. Higher is better.
* **STOI**: intelligibility, 0–1. **PESQ**: wideband quality, 1–4.5.
* **gap atten**: how much quieter the background is in speech pauses.
* **dropout**: % of clearly voiced 20 ms frames whose energy fell more than 10 dB below the
  clean voice (cut syllables).
* **DNSMOS** (Microsoft P.835, no reference needed): SIG = voice quality, BAK = how
  unobtrusive the background is, OVR = overall, each 1–5.
* Presets are scored with the clarity EQ off (`--no-tone`). A linear EQ lowers SI-SDR
  without damaging the voice: on clean speech it scores SI-SDR 7 dB but PESQ 4.53. The
  "+ tone" rows in the full table include it.

## Summary

| Scenario | Input (SI-SDR / STOI) | DFN3 raw | **DPDFNet Balanced** | DPDFNet Voice Priority |
|---|---|---|---|---|
| 1 clean speech | ∞ / 1.00 | 27.5 dB / 1.00, PESQ 4.57 | **34.2 dB / 1.00, PESQ 4.55** | 34.9 dB / 1.00 |
| 2a speech + music | 5.9 / 0.90 | 14.3 / 0.95, drop 1.9 % | **16.9 / 0.97, drop 0.4 %, gaps −36 dB** | 16.6 / 0.97, drop 0.1 %, gaps −21 dB |
| 2b speech + singing | 6.8 / 0.74 | 10.4 / 0.77, **drop 12.7 %** | **15.1 / 0.86, drop 1.4 %, gaps −36 dB** | 15.1 / 0.85, drop 1.0 % |
| 3 speech + effects | 0.7 / 0.82 | 11.7 / 0.91 | **14.4 / 0.95, drop 0.2 %, gaps −36 dB** | 13.7 / 0.93, drop 0 % |
| 4 quiet speech, loud music | −6.2 / 0.64 | 5.1 / 0.75, **drop 25 %** | **8.3 / 0.86, drop 6.0 %** | 7.1 / 0.84, **drop 0.1 %** |
| 5 overlapping voices | 11.7 / 0.95 | 3.8 / 0.78, drop 22 % | 6.2 / 0.84, drop 12 % | 7.1 / 0.87, drop 9 % |
| 7 rapid transitions | −0.7 / 0.86 | 11.7 / 0.94 | **13.7 / 0.96, drop 2.1 %** | 13.0 / 0.95 |

DNSMOS on the input vs the output (DPDFNet Balanced): BAK rises from 1.1–2.4 to 3.2–3.8 and
OVR from 1.1–2.4 to 2.6–3.1 in every scenario that contains a background.

### What is removed when nobody speaks (scenario 6, broken down by source)

| Background alone | Removed by DPDFNet |
|---|---|
| Instrumental music | **−57 dB** (effectively silent) |
| Game and alert sound effects | −22 dB |
| Music with sung vocals | **−11 dB only** |

## Conclusions

1. **DPDFNet-2 48 kHz is the default engine.** It beats DeepFilterNet3 in every mixed
   scenario. The biggest gains are where DFN3 fails: singing in the background (DFN3 cuts
   12.7 % of voiced frames vs DPDFNet's 1.4 %) and quiet speech (25 % vs 6 %). It costs
   20 ms more latency (60 vs 40 ms) and about twice the CPU (2.5 vs 1.3 ms per 10 ms hop).
2. **The post-processing must not fight the model.** An early version used an SNR-driven
   gate, and it raised dropouts on quiet speech to 45 %. The final gate is driven by voice
   level and is off in Balanced, so it adds no dropouts.
3. **The presets work as intended.** *Voice Priority* trades about 15 dB of pause
   suppression for near-zero dropouts. *Maximum Suppression* gives the deepest pauses
   (−45 to −60 dB). *Balanced* keeps pauses about 36 dB down while sounding less "gated".
4. **Hard limits (model, not tuning):**
   * Sung vocals survive (−11 dB).
   * A second talker is partly suppressed (9–12 % dropouts) rather than cleanly kept.
   * Speech 5 dB under music stays intelligible (STOI 0.86) but is audibly thinner.
5. **CPU use:**
   * DPDFNet: 2.5 ms mean and 3.7 ms p99 per 10 ms hop on a 2.1 GHz vCPU, or 25 % of one
     core. This was confirmed under Wine with the actual Windows build.
   * DFN3: 1.3 ms per hop.
   * Memory, measured on Linux as peak RSS minus the 30 s file buffers: about 60 MB for
     DPDFNet and about 50 MB for DFN3.

## Not yet measured (needs the user's Windows PC)

* True end-to-end latency, with a loopback measurement. The current number is an estimate
  built from the actual buffer sizes. The simulated host test measured 67 ms of stream
  delay with DFN3, excluding device buffers.
* CPU, glitches and stability while a demanding game runs, and over an hours-long session.
* Behaviour on real Twitch audio: compressed AAC, streamer microphones, real game mixes.
