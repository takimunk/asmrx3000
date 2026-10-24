#!/usr/bin/env python3
"""Background of the installer window (the DMG): an overexposed sky crossed by power lines over
black ground. The app and Applications hang in the sky (Finder draws icon names in black on a
background picture, whatever the appearance), with a sagging line of red halftone dots between
them showing where to drag; the ground carries the words.

    python3 tools/dmg/make_background.py OUT_DIR

Writes OUT_DIR/background.png (640x420) and OUT_DIR/background@2x.png; scripts/make-dmg.sh joins
them into one HiDPI TIFF. The layout constants are shared with make-dmg.sh (window size, icon
centres): change both together.
"""
import math
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

W, H = 640, 420                      # window content, points
APP, APPS = (170, 178), (470, 178)   # icon centres (make-dmg.sh)
HORIZON = 288                        # where the sky ends

SKY = (233, 230, 222)                # overexposed, a little warm
GROUND = (0, 0, 0)
WIRE = (12, 12, 13)
DOT = (156, 58, 50)                  # muted print red
VOICE = (222, 218, 209)
HINT = (120, 118, 114)

SERIF = "/System/Library/Fonts/NewYorkItalic.ttf"
MONO = Path(__file__).resolve().parents[2] / "apps/macos/Resources/Fonts/CoppiceMono-Regular.ttf"


def catenary(a, b, sag, n=64):
    """Points of a hanging line from a to b, its lowest point `sag` below the chord's middle."""
    (x0, y0), (x1, y1) = a, b
    return [(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t + sag * 4 * t * (1 - t)) for t in (i / n for i in range(n + 1))]


def render(s):
    """The background at scale s (1 or 2)."""
    img = Image.new("RGB", (W * s, H * s), GROUND)
    d = ImageDraw.Draw(img)
    S = lambda v: v * s

    # Sky: flat and blown out, faint grain so it reads as light rather than a fill.
    band, step = 34, 5
    d.rectangle([0, 0, S(W), S(HORIZON + band)], fill=SKY)
    rng = random.Random(7)
    for _ in range(1400 * s * s):
        x, y = rng.uniform(0, S(W)), rng.uniform(0, S(HORIZON + band))
        g = rng.randint(-9, 4)
        d.point((x, y), fill=tuple(max(0, min(255, c + g)) for c in SKY))

    # The horizon dissolves into the ground as an ordered halftone: dots grow towards the black.
    for row, y in enumerate(range(HORIZON, HORIZON + band, step)):
        r = (0.4 + 3.4 * ((y - HORIZON) / band) ** 1.3) * s
        off = (step / 2) if row % 2 else 0
        for x in range(-step, W + step, step):
            cx, cy = S(x + off), S(y)
            d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=GROUND)
    d.rectangle([0, S(HORIZON + band - 1), S(W), S(H)], fill=GROUND)

    # Two poles against the sky, the near one tall on the left, the far one shorter on the right.
    def pole(x, top, arms, w):
        d.rectangle([S(x - w / 2), S(top), S(x + w / 2), S(HORIZON + 6)], fill=WIRE)
        ends = []
        for (y, half) in arms:
            d.rectangle([S(x - half), S(y - 1.6), S(x + half), S(y + 1.6)], fill=WIRE)
            for dx in (-half + 4, -half / 3, half / 3, half - 4):
                d.rectangle([S(x + dx - 1.2), S(y - 5), S(x + dx + 1.2), S(y - 1)], fill=WIRE)   # insulators
                ends.append((x + dx, y - 5))
        return ends

    left = pole(54, -10, [(34, 44)], 6)
    right = pole(596, 22, [(52, 30)], 4)
    for i, (a, b) in enumerate(zip(left, right)):
        pts = catenary(a, b, 26 + 5 * (i % 4))
        d.line([(S(x), S(y)) for x, y in pts], fill=WIRE, width=max(1, round(1.3 * s)))

    # Faint scan lines over the ground only.
    for y in range(S(HORIZON + band), S(H), 3 * s):
        d.line([(0, y), (S(W), y)], fill=(9, 9, 9), width=1)

    # The drag path: a hanging line of halftone dots, growing towards Applications.
    a, b = (APP[0] + 70, APP[1] + 2), (APPS[0] - 70, APPS[1] + 2)
    pts = catenary(a, b, 26, n=22)
    for i, (x, y) in enumerate(pts):
        t = i / (len(pts) - 1)
        r = (1.3 + 2.0 * t) * s
        d.ellipse([S(x) - r, S(y) - r, S(x) + r, S(y) + r], fill=DOT)

    # Words: one lowercase serif line, and the first-launch hint in the app's mono.
    voice = ImageFont.truetype(SERIF, 17 * s)
    text = "drag it into applications"
    tw = d.textlength(text, font=voice)
    d.text(((S(W) - tw) / 2, S(346)), text, font=voice, fill=VOICE)
    mono = ImageFont.truetype(str(MONO), 9 * s)
    hint = "first launch: system settings > privacy & security > open anyway"
    hw = d.textlength(hint, font=mono)
    d.text(((S(W) - hw) / 2, S(384)), hint, font=mono, fill=HINT)
    return img


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    out.mkdir(parents=True, exist_ok=True)
    for s, name in ((1, "background.png"), (2, "background@2x.png")):
        img = render(s)
        img.info["dpi"] = (72 * s, 72 * s)
        img.save(out / name, dpi=(72 * s, 72 * s))
    print(out / "background.png")


if __name__ == "__main__":
    main()
