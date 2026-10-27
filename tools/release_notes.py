#!/usr/bin/env python3
"""Print the GitHub release description for a version: download table + changelog.

    python tools/release_notes.py 0.11.0 > notes.md
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = "https://github.com/Schockgeber/voiceolator"

FILES = [
    ("Windows 10 / 11", "Voiceolator-Setup-Windows-x64.exe", "Installer (recommended)"),
    ("", "Voiceolator-Portable-Windows-x64.zip", "Portable, unzip and run"),
    ("Linux x86_64", "Voiceolator-Linux-x86_64.AppImage", "AppImage: `chmod +x`, then run"),
    ("", "Voiceolator-Linux-x86_64.tar.gz", "Portable; `./install.sh` adds a menu entry"),
    ("Command line", "Voiceolator-CLI-Windows-x64.zip", "Windows"),
    ("", "Voiceolator-CLI-Linux-x86_64.tar.gz", "Linux"),
]


def changelog(ver):
    text = open(os.path.join(ROOT, "CHANGELOG.md"), encoding="utf-8").read()
    m = re.search(rf"^## {re.escape(ver)}\s*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not m:
        sys.exit(f"CHANGELOG.md has no section for {ver}")
    return m.group(1).strip()


def main():
    ver = sys.argv[1].lstrip("v")
    rows = "\n".join(f"| {sys_} | [`{f}`]({REPO}/releases/download/v{ver}/{f}) | {d} |" for sys_, f, d in FILES)
    print(f"""## Download

| System | File | |
|---|---|---|
{rows}

New to Voiceolator? Read the [user guide]({REPO}/blob/v{ver}/docs/USER_GUIDE.md).
Windows may show a SmartScreen warning because the builds are not code-signed:
**More info → Run anyway**. Checksums are in `SHA256SUMS.txt`.

## What's new

{changelog(ver)}
""")


if __name__ == "__main__":
    main()
