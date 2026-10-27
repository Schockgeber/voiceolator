# Contributing to Voiceolator

Thanks for helping! Bug reports, test results from your own setup, ideas and pull
requests are all welcome.

## Reporting a bug

Open an issue with the **Bug report** template and include:

- your system (Windows 10/11 build, or Linux distribution + PipeWire/PulseAudio),
- the Voiceolator version (bottom right of the window),
- what you did, what you expected and what happened,
- the log file:
  - Windows: `%LOCALAPPDATA%\Voiceolator\voiceolator.log` (Options → *Open log folder*)
  - Linux: `~/.config/Voiceolator/voiceolator.log`

Logs contain app and device names, but never audio.

Audio problems are much easier to fix with a short sample. `voiceolator-cli run --app
<browser> --record DIR --seconds 20` saves the original and the processed sound as WAV.
Only share recordings you're allowed to share.

## Development setup

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for building on Windows and Linux,
cross-compiling, packaging and the architecture. In short:

```bash
cargo build --release -p vo-gui -p vo-cli
cargo test --release -p vo-dsp -p vo-audio -p vo-media -p vo-gui
VOICEOLATOR_DEMO=1 target/release/Voiceolator   # UI with simulated audio devices
```

## Pull requests

- One topic per pull request, with a short description of what changes for users.
- Keep the code style of the file you change, and run the tests above.
- If you change audio processing, run the evaluation
  (`tools/eval`, see [docs/EVALUATION.md](docs/EVALUATION.md)) and put the before/after
  numbers in the pull request.
- Keep everything local: no telemetry, accounts or cloud processing. New network access
  must be opt-in and explained to the user (see how the ffmpeg download asks first).
- New dependencies need a permissive licence (MIT, Apache-2.0, BSD, Zlib, …) and an
  entry in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
- User-facing changes go into [CHANGELOG.md](CHANGELOG.md) and, if needed,
  [docs/USER_GUIDE.md](docs/USER_GUIDE.md).

By contributing you agree that your contribution is dual-licensed under MIT OR
Apache-2.0, like the rest of the project.
