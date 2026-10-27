# Security policy

## Supported versions

Only the [latest release](https://github.com/Schockgeber/voiceolator/releases/latest)
receives fixes.

## Reporting a vulnerability

Please **do not** open a public issue. Use GitHub's private reporting instead:
[Report a vulnerability](https://github.com/Schockgeber/voiceolator/security/advisories/new).

Include the affected version, your system, steps to reproduce and the impact. You will
get an answer as soon as possible, and a fix in a new release once the problem is
confirmed.

## What Voiceolator does that matters for security

- It runs with normal user rights. The Windows installer installs per-user and needs no
  administrator rights.
- It changes app volumes (Windows) or audio routing (Linux) while running, and restores
  them on stop or after a crash.
- The only network access is the optional ffmpeg download. It starts only after the
  user agrees, uses HTTPS to fixed URLs, and checks size and SHA-256 against values
  built into the program before ffmpeg is unpacked or run.
- Release files can be checked against `SHA256SUMS.txt` on the release page.
