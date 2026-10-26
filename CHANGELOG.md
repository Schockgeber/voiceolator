# Changelog

## 0.10.0

- **Linux version.** It works with PipeWire (`pipewire-pulse`) and PulseAudio.
  - The chosen app's streams move into a private "Voiceolator-capture" output while
    Voiceolator runs, so only that app is captured and its original sound is hidden
    automatically.
  - Everything goes back on stop.
  - Real latency measurement works on Linux too.
  - Ships as an AppImage, a portable tarball (with `install.sh` for a menu entry) and a
    command-line tarball. It needs glibc 2.31 or newer.
- **Crash watchdog** on both systems: if Voiceolator is killed or crashes, a helper
  restores the browser's sound immediately (Windows volume, Linux routing).
- Release files are renamed by platform (`…-windows-setup.exe`,
  `…-linux-x86_64.AppImage`, …). New `--bundle` option writes one GitHub-ready zip.
- CI builds and tests both platforms.

## 0.9.1

- **Hide the original stream sound** replaces "Mute the original stream sound". Muting an
  app in the Windows mixer also silenced the capture, so you had to re-route the browser
  by hand.
  - Voiceolator now turns the app down to −70 dB and makes up the level exactly in the
    capture. The audio engine works in 32-bit float, so this loses nothing.
  - It checks how Windows applies the volume and never boosts audio that wasn't turned
    down.
  - Volumes are restored on stop, and after a crash at the next start.
  - No Windows sound settings need changing.
- The latency measurement uses the same method, so it works without muting its test tone.
- The command-line tool hides the original by default (`--keep-original` to disable).
- Removed personal names from the license and documentation.

## 0.9.0 (first public beta)

- Desktop app with these controls:
  - source and output selection
  - voice isolation on/off
  - presets: Balanced, Voice Priority, Maximum Suppression
  - sliders: clarity, background suppression, voice preservation
  - volume and mute
  - live meters, latency and CPU display
  - status messages
- Automatic muting of the original app sound in the Windows mixer, with restore on stop
  and after crashes.
- Global hotkeys: Ctrl+Alt+V (isolation on/off), Ctrl+Alt+M (mute).
- Real end-to-end latency measurement (test beeps timed with the audio device clocks).
- Two engines:
  - Quality (DPDFNet-2 48 kHz, ONNX Runtime), the default
  - Light (DeepFilterNet3)
  It falls back to Light automatically when needed.
- Robust audio host:
  - adaptive jitter buffer
  - clock-drift compensation
  - automatic reconnect after device or app changes
  - MMCSS thread priority
- Per-user installer (NSIS) and portable zip.
- Command-line tool: `apps`, `devices`, `run`, `latency`, `bench`, `process`.
