# Changelog

## 0.11.0

- **Files tab: isolate the voice in existing audio and video files.** Drop files on the
  window (or use *Choose files…*, the right-click menu in Explorer, or "Open with" on
  Linux) and press **Isolate voice**.
  - Uses the same voice engine, preset and sliders as the live mode.
  - Writes `name (voice).ext` next to the original (or into a folder you pick). The
    original file is never changed.
  - Videos keep their picture unchanged (no re-encoding) and get the clean voice as sound,
    optionally with the original sound as a second audio track.
  - Audio as WAV, MP3, M4A or FLAC.
  - Runs on several CPU cores (automatic: half of them, at most 4), with progress, time
    left and cancel. It keeps running while the window is hidden.
  - Built-in decoding of WAV, MP3, AAC/M4A/MP4, FLAC, OGG/Vorbis and MKV/WebM (AAC,
    Vorbis, FLAC). Everything else (Opus, AC-3, …) and video output need **ffmpeg**:
    Voiceolator uses an installed ffmpeg, or downloads a pinned, checksum-verified build
    for you after asking (Windows: gyan.dev, Linux: johnvansickle.com).
  - The clean voice stays in sync with the picture (encoder delays are compensated).
  - Command line: `voiceolator-cli process FILE... [--out DIR] [--format wav|mp3|m4a|flac]`,
    `voiceolator-cli ffmpeg --download`.
- **System tray** on Windows and Linux (StatusNotifierItem: KDE, Xfce, Cinnamon, MATE,
  GNOME with the AppIndicator extension, …).
  - Minimize and the close button send Voiceolator to the tray, and the audio keeps
    running. The first close asks once. Both can be switched off in Options.
  - Tray menu: show/hide, start/stop, voice isolation, mute, window layout, quit.
  - Starting Voiceolator again brings the hidden window back.
  - If there is no tray (e.g. GNOME without the extension), the window never hides.
- **Compact window layouts**, from the ⊟ button, the tray menu or Options:
  - **Slim bar** (560×46): start/stop, voice on/off, preset, mute, volume, speech light,
    output level, latency.
  - **Tiny pill** (196×40): start/stop, speech light, voice on/off.
  - Both are borderless, draggable, remember their position, stay on top (switchable),
    and double-click opens the full window.
- Installer: Explorer right-click menu "Isolate the voice with Voiceolator" for common
  audio/video files (optional), and it closes a Voiceolator hidden in the tray before
  updating. `Voiceolator --quit` quits a running copy.
- Linux: the menu entry accepts files ("Open with Voiceolator").
- Release files have fixed names (e.g. `Voiceolator-Setup-Windows-x64.exe`), so the
  download buttons in the README always point to the newest release.
- Fixed: with the light Windows theme, buttons, lists and dialogs were drawn in light
  colours on the dark window. The app now always uses its dark style.

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
