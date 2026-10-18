# /// script
# requires-python = ">=3.11"
# dependencies = ["shapely>=2.0"]
# ///
"""Generates the ASMRX / 3000 menu bar lockup (stacked, nearly square).

Glyphs are built upright from rectangles/polygons in points (strokes and horizontal edges on a
0.5 pt grid so they land on whole pixels at 2x), unioned, corner-cut, then sheared:
ASMRX at 22 deg on top, a horizontal hairline, "3000" at 36 deg with a glitch slice below.
Both rows are justified to the same left/right extents. Writes:
  assets/icon/menubar.svg                                   (source of truth, static + animated preview)
  apps/macos/Sources/ASMRX3000/MenuBarGlyphs.swift          (same rings as CGPoint data)

  uv run tools/icon/make_menubar.py
"""

import math
from pathlib import Path

from shapely import affinity
from shapely.geometry import Polygon, box
from shapely.geometry.polygon import orient
from shapely.ops import unary_union

ROOT = Path(__file__).resolve().parents[2]

CANVAS_W = 24.0
CANVAS_H = 22.0
MARGIN_X = 0.5
T = 1.0                      # stroke, both directions

LETTER_SKEW_DEG = 22.0
LETTER_Y, LETTER_H = 12.0, 8.0   # ASMRX row: bottom, cap height
LETTER_GAP = 0.6
LETTER_REL_W = {"A": 1.0, "S": 1.0, "M": 1.4, "R": 1.0, "X": 1.05}

DIV_Y, DIV_T = 10.5, 0.5         # horizontal hairline

DIGIT_SKEW_DEG = 36.0
DIGIT_Y, DIGIT_H = 2.5, 7.5      # 3000 row
DIGIT_GAP = 0.7
TV_DIGIT = 1.25                 # digit stems: thicker so they survive the 36 deg shear
GLITCH = (1.0, 2.5, 0.5)         # (y0, y1, dx) slice of the digit row shifted right


def chamfer(shape, w, h, tl=0.0, br=0.0):
    cuts = []
    if tl:
        cuts.append(Polygon([(-1, h - tl - 1), (-1, h + 1), (tl + 1, h + 1)]))
    if br:
        cuts.append(Polygon([(w - br - 1, -1), (w + 1, -1), (w + 1, br + 1)]))
    return shape.difference(unary_union(cuts)) if cuts else shape


# --- letters (w, h) -> polygon ------------------------------------------------------------------
def glyph_a(w, h):
    m = round(h * 0.40 * 2) / 2
    g = unary_union([box(0, 0, T, h), box(w - T, 0, w, h), box(0, h - T, w, h), box(0, m, w, m + T)])
    return chamfer(g, w, h, tl=1.6)


def glyph_s(w, h):
    m = round((h - T) / 2 * 2) / 2
    g = unary_union([box(0, h - T, w, h), box(0, m, T, h), box(0, m, w, m + T),
                     box(w - T, 0, w, m + T), box(0, 0, w, T)])
    return chamfer(g, w, h, tl=1.6, br=1.6)


def glyph_m(w, h):
    c = w / 2
    g = unary_union([box(0, 0, T, h), box(w - T, 0, w, h), box(0, h - T, w, h),
                     box(c - T / 2, h * 0.4, c + T / 2, h)])
    return chamfer(g, w, h, tl=1.6)


def glyph_r(w, h):
    m = round(h * 0.40 * 2) / 2
    g = unary_union([box(0, 0, T, h), box(0, h - T, w, h), box(w - T, m, w, h), box(0, m, w, m + T),
                     Polygon([(w - 2 * T, m), (w - T, m), (w, 0), (w - T, 0)])])
    return chamfer(g, w, h, tl=1.6)


def glyph_x(w, h):
    d = T * 1.3
    return unary_union([Polygon([(0, 0), (d, 0), (w, h), (w - d, h)]),
                        Polygon([(w - d, 0), (w, 0), (d, h), (0, h)])])


def digit_3(w, h):
    m = round((h - T) / 2 * 2) / 2
    g = unary_union([box(0, h - T, w, h), box(w * 0.3, m, w, m + T), box(0, 0, w, T),
                     box(w - TV_DIGIT, 0, w, h)])
    return chamfer(g, w, h, tl=0.8, br=1.4)


def digit_0(w, h):
    g = box(0, 0, w, h).difference(box(TV_DIGIT, T, w - TV_DIGIT, h - T))
    return chamfer(g, w, h, tl=1.4, br=1.0)


# --- layout -------------------------------------------------------------------------------------
def row(glyphs, rel, gap, h, k, y0, left, right):
    """Lay glyphs out so that the sheared row spans exactly [left, right]."""
    upright_w = (right - left) - k * h
    unit = (upright_w - gap * (len(glyphs) - 1)) / sum(rel)
    parts, x = [], left
    for make, r in zip(glyphs, rel):
        w = unit * r
        parts.append(affinity.translate(make(w, h), xoff=x))
        x += w + gap
    shape = unary_union(parts)
    return shape


def glitch(shape):
    y0, y1, dx = GLITCH
    big = 1000.0
    return unary_union([
        shape.intersection(box(-big, -big, big, y0)),
        affinity.translate(shape.intersection(box(-big, y0, big, y1)), xoff=dx),
        shape.intersection(box(-big, y1, big, big)),
    ])


def shear(shape, k, y0):
    # x' = x + k*y about the row's own baseline, then move up to y0.
    return affinity.affine_transform(shape, [1, k, 0, 1, 0, y0])


def build():
    kl = math.tan(math.radians(LETTER_SKEW_DEG))
    kd = math.tan(math.radians(DIGIT_SKEW_DEG))
    left, right = MARGIN_X, CANVAS_W - MARGIN_X
    letters = row((glyph_a, glyph_s, glyph_m, glyph_r, glyph_x), [LETTER_REL_W[c] for c in "ASMRX"],
                  LETTER_GAP, LETTER_H, kl, 0, left, right)
    # Leave room on the right of the digit row for the glitch slice.
    digits = row((digit_3, digit_0, digit_0, digit_0), [1, 1, 1, 1], DIGIT_GAP, DIGIT_H, kd, 0,
                 left, right - GLITCH[2])
    digits = glitch(digits)
    # Hairline with ends cut at the letters' angle.
    a = kl * DIV_T
    divider = Polygon([(left, DIV_Y), (right - a, DIV_Y), (right, DIV_Y + DIV_T), (left + a, DIV_Y + DIV_T)])
    shape = unary_union([shear(letters, kl, LETTER_Y), divider, shear(digits, kd, DIGIT_Y)])
    polys = list(shape.geoms) if shape.geom_type == "MultiPolygon" else [shape]
    rings = []
    for p in polys:
        p = orient(p, 1.0)
        rings.append(list(p.exterior.coords)[:-1])
        rings += [list(r.coords)[:-1] for r in p.interiors]
    minx, miny, maxx, maxy = shape.bounds
    assert minx >= 0 and maxx <= CANVAS_W and miny >= 0 and maxy <= CANVAS_H, shape.bounds
    return rings, CANVAS_W, kl


def svg(rings, width):
    def path_d(rs):
        out = []
        for r in rs:
            pts = [f"{x:.3f} {CANVAS_H - y:.3f}" for x, y in r]
            out.append("M" + " L".join(pts) + "Z")
        return " ".join(out)

    d = path_d(rings)
    band = 12
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:g} {CANVAS_H:g}" width="{width * 8:g}" height="{CANVAS_H * 8:g}">
  <!-- ASMRX / 3000 menu bar lockup. Generated by tools/icon/make_menubar.py; do not edit by hand.
       ASMRX {LETTER_SKEW_DEG:g} deg, 3000 {DIGIT_SKEW_DEG:g} deg, {width:g}x{CANVAS_H:g} pt. -->
  <defs>
    <!-- Active state preview: a soft violet band sweeping left to right, 2 s loop. -->
    <linearGradient id="sweep" gradientUnits="userSpaceOnUse" x1="-{band}" y1="0" x2="0" y2="0">
      <stop offset="0" stop-color="#b48cff" stop-opacity="0"/>
      <stop offset="0.5" stop-color="#b48cff" stop-opacity="1"/>
      <stop offset="1" stop-color="#b48cff" stop-opacity="0"/>
      <animateTransform attributeName="gradientTransform" type="translate"
        from="0 0" to="{width + band + 10:g} 0" dur="2s" repeatCount="indefinite"/>
    </linearGradient>
  </defs>
  <path id="wordmark" fill="currentColor" fill-rule="evenodd" d="{d}"/>
  <!-- Remove the next line for the static (idle) mark. -->
  <path fill="url(#sweep)" fill-rule="evenodd" d="{d}"/>
</svg>
'''


def swift(rings, width, k):
    lines = []
    for r in rings:
        pts = ", ".join(f"({x:.3f}, {y:.3f})" for x, y in r)
        lines.append(f"        [{pts}],")
    body = "\n".join(lines)
    return f'''// Generated by tools/icon/make_menubar.py from the same geometry as assets/icon/menubar.svg.
// Do not edit by hand.
import CoreGraphics

enum MenuBarGlyphs {{
    static let size = CGSize(width: {width:g}, height: {CANVAS_H:g})
    /// Italic slope (dx per dy) of the ASMRX row; the sweep band uses it. "3000" is sheared harder.
    static let slant: CGFloat = {k:.4f}
    /// Closed rings in points, y up, origin bottom-left. Fill even-odd.
    static let rings: [[(CGFloat, CGFloat)]] = [
{body}
    ]

    static let path: CGPath = {{
        let p = CGMutablePath()
        for ring in rings {{
            p.addLines(between: ring.map {{ CGPoint(x: $0.0, y: $0.1) }})
            p.closeSubpath()
        }}
        return p
    }}()
}}
'''


def main():
    rings, width, k = build()
    (ROOT / "assets/icon/menubar.svg").write_text(svg(rings, width))
    (ROOT / "apps/macos/Sources/ASMRX3000/MenuBarGlyphs.swift").write_text(swift(rings, width, k))
    print(f"lockup {width:g}x{CANVAS_H:g} pt, {len(rings)} rings")


if __name__ == "__main__":
    main()
