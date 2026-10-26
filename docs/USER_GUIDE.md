# Voiceolator user guide

*For Windows and Linux. Where the steps differ, both are given.*

Voiceolator plays **only the streamer's voice** from a Twitch stream, or any other app,
to your headphones. Background music, game sounds and alerts are removed in real time.
Your game's sound stays exactly as it is.

Everything runs on your PC. Nothing is uploaded and no account is needed.

## Requirements

**Windows**

- Windows 10 version 2004 (May 2020 update) or newer, or Windows 11, 64-bit
- For the **Quality** engine: the free *Microsoft Visual C++ Redistributable (x64)*.
  Most PCs with games already have it. If yours doesn't, Voiceolator tells you and uses
  the **Light** engine instead.
  Download: <https://aka.ms/vs/17/release/vc_redist.x64.exe>

**Linux**

- 64-bit x86 desktop with **PipeWire** (with `pipewire-pulse`, the default on current
  Ubuntu, Fedora, Mint, Debian, Arch and others) or **PulseAudio**
- glibc 2.31 or newer (Ubuntu 20.04, Debian 11, Fedora 32 or newer)
- X11, or Wayland with XWayland (the default)

On both, any modern CPU works. Voiceolator uses about 5–15 % of one CPU core and no
graphics card power for the audio.

## Install

**Windows**

1. Run `Voiceolator-<version>-windows-setup.exe`. You don't need administrator rights.
2. Windows may show **"Windows protected your PC"**. This happens because Voiceolator is
   a new, unsigned open-source program. Click **More info → Run anyway**.
3. Start Voiceolator from the Start menu.

Prefer not to install? Unzip `…-windows-portable.zip` anywhere and run `Voiceolator.exe`.

**Linux**

- **AppImage** (simplest): download `Voiceolator-<version>-linux-x86_64.AppImage`. Make it
  executable (file properties → "Allow executing", or `chmod +x Voiceolator-*.AppImage`)
  and double-click it. If it says FUSE is missing, install `libfuse2`
  (`sudo apt install libfuse2`), or start it with `--appimage-extract-and-run`.
- **Tarball:** extract `Voiceolator-<version>-linux-x86_64.tar.gz` and run `./voiceolator`.
  `./install.sh` adds Voiceolator to your application menu (no root needed), and
  `./install.sh --uninstall` removes it.

## First use (1 minute)

1. **Open the stream** in your browser and let it play.
2. In Voiceolator, under **Listen to**, choose your browser, for example *Firefox
   (playing)*. Click **Refresh** if it isn't in the list yet.
3. Under **Play on**, choose your headphones.
4. Leave **Hide the original stream sound** ticked. You don't need to change any sound
   settings, and you hear only the cleaned voice:
   - **Windows:** Voiceolator turns the browser down to near silence in the volume mixer
     and makes up for it inside the app.
   - **Linux:** the browser plays into Voiceolator's private "Voiceolator-capture" output
     instead of your speakers.

   When you press Stop or close Voiceolator, everything goes back as it was, even if
   Voiceolator crashes.
5. Press **Start**. Now start your game.

## Controls

| Control | What it does |
|---|---|
| **ON · voice only / OFF · original sound** | Switch between the cleaned voice and the original, so you can compare them |
| **Balanced** | Strong removal of music and effects with a natural sound. Best for most streams |
| **Voice Priority** | Never cuts quiet words, but lets a little more background through. Good for quiet or soft-spoken streamers |
| **Maximum Suppression** | Removes as much background as possible. Pauses are near-silent |
| **Speech clarity** | Makes the voice brighter and removes low rumble |
| **Background suppression** | How strongly music and effects are removed |
| **Voice preservation** | How much quiet or overlapping speech is protected |
| **Hide the original stream sound** | Stops you hearing the stream twice. No Windows settings needed |
| **Volume / Mute** | Voiceolator's own output level. It doesn't affect your game |
| **Monitor** | Input level, how much speech is detected, output level, delay and CPU use |
| **Measure latency** | Plays about 10 short beeps through the whole path and measures the real delay |

### Hotkeys (Windows; they work while you're in a game)

- **Ctrl + Alt + V**: voice isolation on/off
- **Ctrl + Alt + M**: mute/unmute Voiceolator

### Options

- **Engine**:
  - **Quality** (default): best with music, about 60 ms of processing delay.
  - **Light**: less CPU and about 40 ms delay, but weaker when music plays.
- **Buffer**: raise it if you hear crackles. It adds a little delay.
- **Auto-level quiet streamers**: evens out the volume.
- **Keep window on top**, **Start listening automatically**.

## What to expect

**Works very well:**

- Instrumental background music: it practically disappears (−57 dB in our tests).
- Game sounds, explosions, alerts and sound effects (about −22 dB).
- Speech stays clear and natural.

**Known limits.** Every real-time voice filter has these:

- **Songs with singing.** Singing is a human voice, so it is only partly removed (about
  −11 dB).
- **Other voices** (game characters, a co-host, text-to-speech donations) are voices too.
  They are usually kept, sometimes partly reduced.
- **Very quiet speech under very loud music** stays understandable, but can sound a bit
  thinner. Use **Voice Priority**.
- **Delay.** The cleaned voice comes about 80–110 ms after the original (Light engine: 60–90 ms). Press **Measure latency** to see the real value on your PC. For listening
  while you play, this doesn't matter. When you watch the video closely, lips can be
  slightly out of sync.
- **Whole-browser capture.** Voiceolator captures the whole browser, not a single tab.
  Other sounds in that browser (another tab playing a video) go through it too. Tip: use
  a separate browser for Twitch.

## Troubleshooting

**Windows**

| Problem | Fix |
|---|---|
| My browser is not in the list | Start playing the stream first, then click **Refresh**. Only apps that have produced sound appear |
| I hear the stream twice | Tick **Hide the original stream sound** (on by default) |
| I changed the browser's output device in Windows earlier | You don't need that any more. In *Settings → System → Sound → Volume mixer*, set the browser's output back to **Default** |
| The browser is very quiet after using Voiceolator | Voiceolator restores the browser's volume when you stop it, and at the next start if it crashed. If it's still quiet, raise the browser in the Windows volume mixer |
| No sound at all | Check **Play on** is your headphones, the volume isn't muted, and the **Input** meter moves. If Input stays empty, the stream may be paused, or you picked the wrong app |
| Crackling or dropouts | Options → raise **Buffer** to 40 ms. Close other heavy programs. Switch to the **Light** engine |
| "Quality engine needs the Microsoft Visual C++ Redistributable" | Install it from <https://aka.ms/vs/17/release/vc_redist.x64.exe>, then restart Voiceolator |
| "process loopback capture is not available" | Your Windows is older than version 2004. Run Windows Update |
| Bluetooth headphones feel very delayed | Bluetooth itself adds 100–250 ms. Wired headphones are much faster |
| The window doesn't open | Update your graphics driver (OpenGL 2.0 or newer is needed) |

**Linux**

| Problem | Fix |
|---|---|
| "no PulseAudio/PipeWire sound server reachable" | Install and start `pipewire-pulse` (or PulseAudio). Voiceolator talks to the sound server through its PulseAudio interface |
| The browser is not in the list | Start playing first, then click **Refresh**. Flatpak and Snap browsers use the same sound server, so they should appear too |
| The browser stays silent after a crash | This normally fixes itself within a second, because a small watchdog cleans up after a crash. Otherwise start Voiceolator once, or run `pactl unload-module $(pactl list short modules \| grep voiceolator_capture \| cut -f1)` |
| The AppImage doesn't start | `chmod +x` it, install `libfuse2`, or run it with `--appimage-extract-and-run` |
| The window doesn't open | Make sure OpenGL works (`glxinfo -B`) and that you have X11 or XWayland |

**Files**

- Windows log: `%LOCALAPPDATA%\Voiceolator\voiceolator.log` (Options → *Open log folder*).
- Windows settings: `%APPDATA%\Voiceolator\config.json`.
- Linux settings and log: `~/.config/Voiceolator/`.

Delete the settings file to reset everything.

## Privacy

Voiceolator does not connect to the internet. Audio is processed only in memory on your
PC and is never saved.

## Uninstall

- **Windows:** *Settings → Apps → Voiceolator → Uninstall*, or the uninstaller in the
  Start menu.
- **Linux:** delete the AppImage, or run `./install.sh --uninstall`.
