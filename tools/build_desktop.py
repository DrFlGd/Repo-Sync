#!/usr/bin/env python3
"""Prepare the desktop app's front end for Tauri.

    python3 tools/build_desktop.py --out build/desktop [--fetch-fonts]

Copies web/ to <out>/ui (tauri.conf.json's frontendDist), with THIRD_PARTY.md and
build-info.json (date, commit) for the About page. The app works offline, so the
Google Fonts lines are dropped from index.html: with --fetch-fonts (CI) Overpass
and Overpass Mono (SIL OFL, from google/fonts) are bundled instead, otherwise
system fonts are used.
"""
import argparse
import datetime
import json
import os
import shutil
import subprocess
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ap = argparse.ArgumentParser()
ap.add_argument("--out", type=Path, default=ROOT / "build/desktop")
ap.add_argument("--fetch-fonts", action="store_true", help="bundle the fonts instead of loading them from Google Fonts")
a = ap.parse_args()

ui = a.out / "ui"
if ui.exists():
    shutil.rmtree(ui)
shutil.copytree(ROOT / "web", ui)
shutil.copy2(ROOT / "THIRD_PARTY.md", ui / "THIRD_PARTY.md")


def commit():
    sha = os.environ.get("GITHUB_SHA")
    if not sha:
        try:
            sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            return None
    return sha[:7] or None


(ui / "build-info.json").write_text(json.dumps({
    "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d"),
    "commit": commit(),
}, indent=2) + "\n", encoding="utf-8")

FONT_URL = "https://raw.githubusercontent.com/google/fonts/main/ofl/"
FONTS = [
    ("Overpass", "overpass", "Overpass%5Bwght%5D.ttf", "100 900"),
    ("Overpass Mono", "overpassmono", "OverpassMono%5Bwght%5D.ttf", "300 700"),
]
index = ui / "index.html"
lines = [l for l in index.read_text(encoding="utf-8").splitlines() if "fonts.googleapis.com" not in l and "fonts.gstatic.com" not in l]
font_css = ""
if a.fetch_fonts:
    fonts = ui / "vendor/fonts"
    fonts.mkdir(parents=True, exist_ok=True)
    faces = []
    for family, folder, remote, weights in FONTS:
        name = family.replace(" ", "")
        get = lambda path: urllib.request.urlopen(FONT_URL + path, timeout=60).read()
        (fonts / f"{name}-Variable.ttf").write_bytes(get(f"{folder}/{remote}"))
        (fonts / f"{name}-OFL.txt").write_bytes(get(f"{folder}/OFL.txt"))
        faces.append(f"@font-face {{ font-family: '{family}'; src: url('{name}-Variable.ttf') format('truetype'); font-weight: {weights}; font-display: swap; }}\n")
    (fonts / "fonts.css").write_text("".join(faces))
    font_css = '  <link rel="stylesheet" href="vendor/fonts/fonts.css">'
out = []
for l in lines:
    if font_css and 'href="styles.css"' in l:
        out.append(font_css)
    out.append(l)
index.write_text("\n".join(out) + "\n", encoding="utf-8")
size = sum(f.stat().st_size for f in ui.rglob("*") if f.is_file())
print(f"ui {size / 1e6:.1f} MB -> {ui}")
