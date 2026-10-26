# Developing Voiceolator

## Layout

```
crates/vo-dsp     platform-independent DSP: engines, pipeline, presets, drift resampler
crates/vo-audio   audio host + Windows WASAPI backend (capture, render, mute, latency)
crates/vo-gui     desktop app (eframe/egui, glow renderer); demo backend on non-Windows
crates/vo-cli     command-line tool (also used for testing on real hardware)
crates/vo-eval    offline runner of the exact real-time pipeline over WAV files
tools/eval        test-set builder + scoring (SI-SDR, STOI, PESQ, DNSMOS, dropouts)
tools/fetch_deps.py   downloads DPDFNet model + onnxruntime.dll (pinned, checksummed)
tools/package.py      builds installer + portable zip
installer/        NSIS script
docs/             user guide, evaluation, this file
```

## Build

On Windows (MSVC):

```
cargo build --release -p vo-gui -p vo-cli
python tools/fetch_deps.py --out target/release
target\release\Voiceolator.exe
```

Cross-compiling from Linux (this is how the releases are built):

```
sudo apt install mingw-w64 nsis libpulse-dev
rustup target add x86_64-pc-windows-gnu
# ~/.cargo/config.toml:  [target.x86_64-pc-windows-gnu] linker = "x86_64-w64-mingw32-gcc"
pip install markdown
python tools/package.py --bundle   # -> dist/: Windows + Linux downloads, SHA256SUMS.txt, GitHub-ready zip
```

The exe needs only Windows system DLLs. `onnxruntime.dll` is loaded at runtime. If it's
missing, or its VC++ runtime is missing, the app falls back to DeepFilterNet3.

`Cargo.lock` must keep `tract-* = 0.21.4`. libDF (DeepFilterNet) does not compile against
newer tract.

## Linux

Native build on Linux:

```
sudo apt install libpulse-dev
cargo build --release -p vo-gui -p vo-cli
python tools/fetch_deps.py --out target/release --linux-ort
target/release/Voiceolator
```

Release builds target glibc 2.31 with zig:

```
pip install ziglang
cargo install cargo-zigbuild
python tools/package.py --only linux --appimagetool /path/to/appimagetool
```

How capture works on Linux (`crates/vo-audio/src/linux`):

1. Voiceolator loads `module-null-sink` (`voiceolator_capture`) and moves the chosen
   app's sink-inputs into it.
2. It records `voiceolator_capture.monitor` with libpulse-simple.
3. It plays the result to the selected sink.
4. On stop, the streams move back and the module is unloaded. A watchdog process does
   the same after a crash.

This works on PulseAudio and on PipeWire through `pipewire-pulse`.

`VOICEOLATOR_DEMO=1` starts the GUI with simulated devices. Add
`VOICEOLATOR_DEMO_WAV=file.wav` to process a file.

## Tests

```
cargo test --release -p vo-dsp -p vo-audio -p vo-gui
```

- `vo-dsp`: filters, limiter, delay line, drift resampler (0.1 % clock skew over 60 s).
- `vo-audio/tests/host_sim.rs`: the full 3-thread host with simulated real-time devices.
  It checks streaming, latency bound, clock-drift absorption and recovery after a device
  disconnect.
- `vo-gui`: configuration compatibility and preset mapping.
- `vo-audio` duck compensator: the gain logic used to hide the original sound on Windows.

Hardware tests (Windows and Linux):

```
voiceolator-cli bench                          # model CPU cost
voiceolator-cli apps / devices
voiceolator-cli run --app firefox --output "Headphones" --record ab-test   # original sound hidden by default
voiceolator-cli latency --output "Headphones"  # real end-to-end delay (plays beeps)
```

## Evaluation

```
python tools/eval/make_testset.py --data DATA --out TESTSET   # EARS + MUSDB18 previews + ESC-50
python tools/eval/evaluate.py --testset TESTSET --bin target/release/vo-eval \
    --dnsmos DATA/dnsmos --ort libonnxruntime.so --out OUT
```

Results and method are in [EVALUATION.md](EVALUATION.md). Test audio is not
redistributed.

## Adding a voice model

1. Implement `vo_dsp::VoiceEngine` (48 kHz mono, fixed hop, report `latency_samples`).
2. Add a variant to `EngineKind` and to `create_engine`.
3. Run the evaluation and compare against DPDFNet before making it a default.
