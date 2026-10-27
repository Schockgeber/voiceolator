#!/usr/bin/env python3
"""Draw the README graphics (banner, logo, download buttons) as self-contained SVGs.

    python tools/readme_graphics.py

Text is converted to vector outlines (Inter, SIL Open Font License, fetched once from
Google Fonts into target/fonts), so the images look identical everywhere and need no
web fonts. The Linux logo is the CC0 "Linux" icon from Simple Icons. Writes
docs/assets/*.svg. Needs: pip install fonttools
"""
import math
import os
import re
import urllib.request

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "docs", "assets")
CACHE = os.path.join(ROOT, "target", "fonts")

TEAL = "#2ED6C4"
VIOLET = "#A084FF"
BLUE = "#6AA8FF"


def fetch(url, path):
    if not os.path.exists(path):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "wb").write(urllib.request.urlopen(url).read())
    return path


def font(weight):
    path = os.path.join(CACHE, f"Inter-{weight}.ttf")
    if not os.path.exists(path):
        css = urllib.request.urlopen(f"https://fonts.googleapis.com/css2?family=Inter:wght@{weight}").read().decode()
        fetch(re.search(r"url\((https://[^)]+\.ttf)\)", css).group(1), path)
    return TTFont(path)


class Text:
    """Lays out a string with the font's pair kerning (GPOS) and returns an SVG path."""

    def __init__(self, weight):
        self.f = font(weight)
        self.gs = self.f.getGlyphSet()
        self.cmap = self.f.getBestCmap()
        self.upem = self.f["head"].unitsPerEm
        self.pairs = {}
        self.classes = []
        if "GPOS" in self.f:
            for lk in self.f["GPOS"].table.LookupList.Lookup:
                for st in lk.SubTable:
                    if lk.LookupType == 9:
                        if st.ExtensionLookupType != 2:
                            continue
                        st = st.ExtSubTable
                    elif lk.LookupType != 2:
                        continue
                    cov = st.Coverage.glyphs
                    if st.Format == 1:
                        for g1, ps in zip(cov, st.PairSet):
                            for r in ps.PairValueRecord:
                                v = getattr(r.Value1, "XAdvance", 0) if r.Value1 else 0
                                if v:
                                    self.pairs.setdefault((g1, r.SecondGlyph), v)
                    elif st.Format == 2:
                        self.classes.append((set(cov), st.ClassDef1.classDefs, st.ClassDef2.classDefs, st.Class1Record))

    def kern(self, a, b):
        v = self.pairs.get((a, b))
        if v is not None:
            return v
        for cov, c1, c2, rec in self.classes:
            if a in cov:
                r = rec[c1.get(a, 0)].Class2Record[c2.get(b, 0)]
                x = getattr(r.Value1, "XAdvance", 0) if r.Value1 else 0
                if x:
                    return x
        return 0

    def _advances(self, s, tracking):
        g = [self.cmap[ord(c)] for c in s]
        x = 0
        for i, n in enumerate(g):
            yield n, x
            x += self.gs[n].width + tracking * self.upem
            if i + 1 < len(g):
                x += self.kern(n, g[i + 1])
        yield None, x

    def width(self, s, size, tracking=0.0):
        return list(self._advances(s, tracking))[-1][1] * size / self.upem

    def path(self, s, x, y, size, tracking=0.0):
        k = size / self.upem
        pen = SVGPathPen(self.gs, ntos=lambda v: ("%.1f" % v).rstrip("0").rstrip("."))
        for n, cx in self._advances(s, tracking):
            if n is not None:
                self.gs[n].draw(TransformPen(pen, (k, 0, 0, -k, x + cx * k, y)))
        return pen.getCommands()


def icon(x, y, s, idp=""):
    """The app icon: waveform bars on a dark rounded square (matches icon.png)."""
    r = s * 0.22
    heights = [0.16, 0.26, 0.20, 0.42, 0.62, 0.80, 0.62, 0.50, 0.26, 0.34, 0.16]
    colors = ["#5b6178"] * 3 + ["#2ED6C4", "#43C7E0", "#6AA8FF", "#7F9BFF", "#A084FF"] + ["#5b6178"] * 3
    n = len(heights)
    bw = s * 0.045
    gap = (s * 0.72 - n * bw) / (n - 1)
    x0 = x + s * 0.14
    out = [
        f'<rect x="{x}" y="{y}" width="{s}" height="{s}" rx="{r:.1f}" fill="url(#iconbg{idp})"/>',
        f'<rect x="{x + 0.5}" y="{y + 0.5}" width="{s - 1}" height="{s - 1}" rx="{r:.1f}" fill="none" stroke="#ffffff" stroke-opacity="0.08"/>',
    ]
    for i, (h, c) in enumerate(zip(heights, colors)):
        hh = s * h
        bx = x0 + i * (bw + gap)
        out.append(f'<rect x="{bx:.1f}" y="{y + s / 2 - hh / 2:.1f}" width="{bw:.1f}" height="{hh:.1f}" rx="{bw / 2:.1f}" fill="{c}"/>')
    defs = f'<linearGradient id="iconbg{idp}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#23263a"/><stop offset="1" stop-color="#14161f"/></linearGradient>'
    return defs, "\n".join(out)


def write(name, w, h, defs, body, title):
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{title}">\n'
        f"<title>{title}</title>\n<defs>{defs}</defs>\n{body}\n</svg>\n"
    )
    os.makedirs(OUT, exist_ok=True)
    open(os.path.join(OUT, name), "w").write(svg)
    print("wrote", os.path.join("docs", "assets", name), f"({len(svg) // 1024} KB)")


def banner(t800, t600, t400):
    W, H = 1280, 360
    d_icon, b_icon = icon(72, 70, 220)
    title, tag, sub = "Voiceolator", "Hear only the streamer.", "Real-time AI voice isolation for Twitch streams and video files"
    tx = 340
    right = tx + max(t800.width(title, 96, -0.02), t600.width(tag, 38), t400.width(sub, 23))
    bars = []
    x, i = right + 60, 0
    span = max(1.0, W - 60 - (right + 60))
    while x < W - 60:
        p = (x - right - 60) / span
        env = math.exp(-((p - 0.5) ** 2) / 0.06)
        hgt = 24 + 150 * env * (0.55 + 0.45 * abs(math.sin(i * 1.7)))
        col = TEAL if p < 0.35 else (BLUE if p < 0.65 else VIOLET)
        bars.append(f'<rect x="{x:.1f}" y="{180 - hgt / 2:.1f}" width="7" height="{hgt:.1f}" rx="3.5" fill="{col}" fill-opacity="{0.15 + 0.35 * env:.2f}"/>')
        x += 15
        i += 1
    defs = (
        d_icon
        + '<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0f1118"/><stop offset="1" stop-color="#1b1e2b"/></linearGradient>'
        + f'<radialGradient id="glow" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="{TEAL}" stop-opacity="0.22"/><stop offset="1" stop-color="{TEAL}" stop-opacity="0"/></radialGradient>'
        + f'<linearGradient id="tag" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{TEAL}"/><stop offset="1" stop-color="{VIOLET}"/></linearGradient>'
    )
    body = "\n".join(
        [
            f'<rect width="{W}" height="{H}" rx="28" fill="url(#bg)"/>',
            f'<rect x="0.5" y="0.5" width="{W - 1}" height="{H - 1}" rx="28" fill="none" stroke="#ffffff" stroke-opacity="0.07"/>',
            '<circle cx="182" cy="180" r="210" fill="url(#glow)"/>',
            *bars,
            b_icon,
            f'<path d="{t800.path(title, tx, 178, 96, -0.02)}" fill="#ffffff"/>',
            f'<path d="{t600.path(tag, tx + 2, 236, 38)}" fill="url(#tag)"/>',
            f'<path d="{t400.path(sub, tx + 2, 280, 23)}" fill="#a9b0c6"/>',
        ]
    )
    write("banner.svg", W, H, defs, body, "Voiceolator: hear only the streamer")


def windows_logo(x, y, s, col):
    g = s * 0.08
    q = (s - g) / 2
    return "".join(f'<rect x="{x + dx:.1f}" y="{y + dy:.1f}" width="{q:.1f}" height="{q:.1f}" rx="1.5" fill="{col}"/>' for dx in (0, q + g) for dy in (0, q + g))


def linux_logo(x, y, s, col):
    svg = open(fetch("https://cdn.jsdelivr.net/npm/simple-icons@13.21.0/icons/linux.svg", os.path.join(CACHE, "linux.svg"))).read()
    d = re.search(r' d="([^"]+)"', svg).group(1)
    return f'<path transform="translate({x} {y}) scale({s / 24:.4f})" d="{d}" fill="{col}"/>'


def list_logo(x, y, s, col):
    out = []
    for i in range(3):
        yy = y + s * (0.12 + 0.32 * i)
        out.append(f'<rect x="{x:.1f}" y="{yy:.1f}" width="{s * 0.2:.1f}" height="{s * 0.2:.1f}" rx="2" fill="{col}"/>')
        out.append(f'<rect x="{x + s * 0.32:.1f}" y="{yy + s * 0.04:.1f}" width="{s * 0.68:.1f}" height="{s * 0.12:.1f}" rx="2" fill="{col}"/>')
    return "".join(out)


def button(name, t600, t400, title, subtitle, logo, fill_a, fill_b, fg, sub_fg, stroke=None, arrow=True):
    W, H = 372, 76
    tw = max(t600.width(title, 20), t400.width(subtitle, 13.5))
    assert 74 + tw < (W - 60 if arrow else W - 16), f"{name}: text too wide ({tw:.0f})"
    defs = f'<linearGradient id="f" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{fill_a}"/><stop offset="1" stop-color="{fill_b}"/></linearGradient>'
    parts = [
        f'<rect x="1" y="1" width="{W - 2}" height="{H - 2}" rx="16" fill="url(#f)"' + (f' stroke="{stroke}" stroke-width="1.5"' if stroke else "") + "/>",
        logo(24, 22, 32, fg),
        f'<path d="{t600.path(title, 74, 36, 20)}" fill="{fg}"/>',
        f'<path d="{t400.path(subtitle, 74, 57, 13.5)}" fill="{sub_fg}"/>',
    ]
    if arrow:
        ax, ay = W - 40, 36
        parts.append(
            f'<g fill="none" stroke="{fg}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round">'
            f'<path d="M{ax} {ay - 11} V{ay + 5}"/><path d="M{ax - 7} {ay - 2} L{ax} {ay + 5} L{ax + 7} {ay - 2}"/><path d="M{ax - 9} {ay + 13} H{ax + 9}"/></g>'
        )
    write(name, W, H, defs, "\n".join(parts), title)


def main():
    t800, t600, t400 = Text(800), Text(600), Text(400)
    banner(t800, t600, t400)
    d, b = icon(0, 0, 256)
    write("logo.svg", 256, 256, d, b, "Voiceolator")
    button("download-windows.svg", t600, t400, "Download for Windows", "Installer · Windows 10 & 11 · 64-bit", windows_logo, "#3ee6d3", "#1fbcab", "#06201d", "#0b3a35")
    button("download-linux.svg", t600, t400, "Download for Linux", "AppImage · x86_64 · any distro", linux_logo, "#b6a1ff", "#8a68f5", "#140b33", "#2a1c5c")
    button("all-downloads.svg", t600, t400, "All downloads", "Portable, command line, older versions", list_logo, "#23263a", "#181a26", "#e8ebf5", "#9aa1b8", stroke="#3a3f58", arrow=False)


if __name__ == "__main__":
    main()
