# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools", "shapely>=2", "pillow"]
# ///
"""Coppice Mono: a pixel-grid literary serif for terminals (PIXEL sets the bitmap size).
Latin, figures, punctuation and the full Russian Cyrillic (А–я, Ёё, №, «», —, …, −).

Every glyph is a centreline skeleton swept by a fixed elliptical nib (broad
vertical stress: fat stems, hairline joins), plus hairline slab serifs.
Run:  uv run tools/font/coppice.py [outdir]     -> CoppiceMono-Regular.ttf
"""
import math
import sys
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from shapely import affinity
from shapely.geometry import MultiPoint, Point, Polygon
from shapely.geometry.polygon import orient
from shapely.ops import unary_union

UPM, XH, CAP, ASC, DESC = 1000, 480, 700, 745, -225
T, H = 58, 26                        # stem and hairline thickness
A, B = T / 2, H / 2
N = (A, B)                           # text nib: vertical stress
HR = (B, B)                          # hairline
HV = (26, 13)                        # heavy diagonal
MD = (22, 13)                        # medium diagonal
CY, RY = 240, 238                    # round lowercase: centre y / centreline radius
SLANT, NARROW = 11, 0.91
BUF = 21     # every stroke is fattened by this after sweeping: stems ~90, hairlines ~58 (low contrast, even colour)
MONO = True   # fixed 600-unit cell: terminal-style hybrid
PIXEL = 38      # bitmap grid in font units (UPM 1000): the one knob for how pixelated the outlines look (570 cell = 15 px)
PIXEL_COVER = 0.45   # a grid cell is inked when at least this fraction of it is covered by the smooth outline

# ───────────────────────── geometry ─────────────────────────

def ell(cx, cy, a, b, n=26):
    return [(cx + a * math.cos(2 * math.pi * i / n), cy + b * math.sin(2 * math.pi * i / n)) for i in range(n)]

def sweep(pts, pen, k0=1.0, k1=1.0, bulge=None):
    a, b = pen
    cum = [0.0]
    for p, q in zip(pts, pts[1:]):
        cum.append(cum[-1] + math.dist(p, q))
    total = cum[-1] or 1.0
    shapes, prev = [], None
    for (x, y), c in zip(pts, cum):
        t = c / total
        k = k0 + (k1 - k0) * t
        if bulge is not None:
            k = bulge + (1 - bulge) * math.sin(math.pi * t)
        cur = ell(x, y, max(a * k, b), b)
        if prev is not None:
            shapes.append(MultiPoint(prev + cur).convex_hull)
        prev = cur
    return unary_union(shapes)

def line(p, q):
    return [p, q]

def bez(p0, p1, p2, p3, step=7):
    L = math.dist(p0, p1) + math.dist(p1, p2) + math.dist(p2, p3)
    n = max(6, int(L / step))
    out = []
    for i in range(n + 1):
        t = i / n; u = 1 - t
        out.append((u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                    u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]))
    return out

def arc(cx, cy, rx, ry, a0, a1, e=2.0, step=7):
    n = max(8, int(abs(math.radians(a1 - a0)) * (rx + ry) / 2 / step))
    out = []
    for i in range(n + 1):
        t = math.radians(a0 + (a1 - a0) * i / n)
        c, s = math.cos(t), math.sin(t)
        out.append((cx + rx * math.copysign(abs(c) ** (2 / e), c), cy + ry * math.copysign(abs(s) ** (2 / e), s)))
    return out

def cat(*paths):
    out = list(paths[0])
    for p in paths[1:]:
        out += p[1:] if math.dist(out[-1], p[0]) < 1.5 else p
    return out

class G:
    def __init__(self, ital, xf=1.0):
        self.parts, self.ital, self.xf = [], ital, xf

    def s(self, pts, pen=N, k0=1.0, k1=1.0, bulge=None):
        self.parts.append(sweep([(x * self.xf, y) for x, y in pts], pen, k0, k1, bulge))

    def poly(self, pts): self.parts.append(Polygon([(x * self.xf, y) for x, y in pts]))
    def rect(self, x0, y0, x1, y1): self.poly([(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
    def dot(self, x, y, r): self.parts.append(Point(x * self.xf, y).buffer(max(r - 12, 8), cap_style='square'))  # square terminals
    def geom(self): return unary_union(self.parts)

    def stem(self, x, y0, y1, pen=N, k0=1.0, k1=1.0):
        self.s(line((x, y0 + pen[1]), (x, y1 - pen[1])), pen, k0, k1)

    # roman hairline serifs ------------------------------------------------
    def foot(self, x, l=92, r=92):
        if self.ital: return
        self.rect(x - l, 0, x + r, H)
        if l > A: self.poly([(x - A, H), (x - A - 26, H), (x - A, H + 36)])
        if r > A: self.poly([(x + A, H), (x + A + 26, H), (x + A, H + 36)])

    def top(self, x, y, l=92, r=92):
        self.rect(x - l, y - H, x + r, y)
        if l > A: self.poly([(x - A, y - H), (x - A - 26, y - H), (x - A, y - H - 36)])
        if r > A: self.poly([(x + A, y - H), (x + A + 26, y - H), (x + A, y - H - 36)])

    def flag(self, x, y, w=None):
        w = w or (56 if self.ital else 78)
        self.rect(x - w, y - H, x, y)
        self.poly([(x - A, y - H), (x - A - 26, y - H), (x - A, y - H - 38)])
        if not self.ital: self.rect(x - w, y - 36, x - w + H, y)

    def tail(self, x):
        pts = cat(bez((x, 110), (x, 50), (x + 14, 11), (x + 60, 11)), bez((x + 60, 11), (x + 100, 11), (x + 124, 40), (x + 142, 82)))
        self.s(pts, N, 1.0, 0.3)

    def leg(self, x, ytop, l=92, r=92, tail=True):
        """Stem that lands on the baseline: serif in roman, exit stroke (or nothing) in italic."""
        if self.ital:
            if tail:
                self.stem(x, 100, ytop); self.tail(x)
            else:
                self.stem(x, 0, ytop)
        else:
            self.stem(x, 0, ytop); self.foot(x, l, r)

GL = {}
def glyph(*chars):
    def deco(f):
        for c in chars: GL[c] = f
        return f
    return deco

# ───────────────────────── lowercase ─────────────────────────

def arch(g, L, R, y0=300, e=2.1):
    g.s(arc((L + R) / 2, y0, (R - L) / 2, XH - 2 - y0, 180, 0, e))

@glyph('n')
def _(g):
    L, R = 60, 420
    g.stem(L, 0, XH); arch(g, L, R); g.flag(L, XH)
    g.leg(L, 0, tail=False) if False else (g.foot(L) if not g.ital else None)
    g.leg(R, 300)

@glyph('h')
def _(g):
    L, R = 60, 420
    g.stem(L, 0, ASC); arch(g, L, R); g.flag(L, ASC); g.foot(L); g.leg(R, 300)

@glyph('m')
def _(g):
    L, M, R = 50, 250, 450
    g.stem(L, 0, XH); g.flag(L, XH, 60); g.foot(L, 60, 40)
    arch(g, L, M); arch(g, M, R)
    g.stem(M, 0, 300); g.stem(R, 0, 300); g.foot(R, 50, 50)

@glyph('r')
def _(g):
    L = 60
    g.stem(L, 0, XH); g.flag(L, XH, 100); g.foot(L, 110, 150)
    g.s(bez((L, 300), (L + 40, 430), (150, 474), (245, 470)), N, 1.0, 0.5)
    g.dot(268, 440, 38)

@glyph('i')
def _(g):
    L = 60
    g.stem(L, 0, XH); g.flag(L, XH, 100); g.foot(L, 140, 140) if not g.ital else g.tail(L)
    g.dot(L + (6 if g.ital else 0), 628, 48)

@glyph('l')
def _(g):
    L = 60
    g.stem(L, 0, ASC); g.flag(L, ASC, 100); g.foot(L, 140, 140) if not g.ital else g.tail(L)

@glyph('u')
def _(g):
    L, R = 60, 420
    g.stem(L, 180, XH); g.flag(L, XH)
    g.s(arc(240, 180, 180, 178, 180, 360, 2.1))
    g.stem(R, 0, XH); g.flag(R, XH); g.foot(R, 20, 92)
    if g.ital: g.tail(R)

@glyph('o')
def _(g): g.s(arc(258, CY, 216, RY, 0, 360, 2.12))

@glyph('c')
def _(g):
    g.s(arc(250, CY, 208, RY, 40, 322, 2.12), N, 1.0, 0.85)
    g.dot(414, 392, 37)

@glyph('e')
def _(g):
    g.s(arc(250, CY, 210, RY, 0, 326, 2.12), N, 1.0, 0.8)
    g.s(line((42, CY + 4), (460, CY + 4)), HR)

@glyph('d')
def _(g):
    cx, rx = 250, 205
    g.s(arc(cx, CY, rx, RY, 0, 360, 2.12))
    g.stem(cx + rx, 0 if not g.ital else 100, ASC); g.top(cx + rx, ASC, 78, 0) if False else g.flag(cx + rx, ASC)
    g.foot(cx + rx, 70, 92)
    if g.ital: g.tail(cx + rx)

@glyph('b')
def _(g):
    L, rx = 60, 205
    g.s(arc(L + rx, CY, rx, RY, 0, 360, 2.12))
    g.stem(L, 0, ASC); g.flag(L, ASC); g.foot(L)

@glyph('p')
def _(g):
    L, rx = 60, 205
    g.s(arc(L + rx, CY, rx, RY, 0, 360, 2.12))
    g.stem(L, DESC, XH); g.flag(L, XH)
    if not g.ital:
        g.rect(L - 92, DESC, L + 92, DESC + H)
        g.poly([(L - A, DESC + H), (L - A - 26, DESC + H), (L - A, DESC + H + 36)])
        g.poly([(L + A, DESC + H), (L + A + 26, DESC + H), (L + A, DESC + H + 36)])

@glyph('q')
def _(g):
    cx, rx = 250, 205
    R = cx + rx
    g.s(arc(cx, CY, rx, RY, 0, 360, 2.12))
    g.stem(R, DESC, XH)
    if not g.ital:
        g.rect(R - 92, DESC, R + 92, DESC + H)
        g.poly([(R - A, DESC + H), (R - A - 26, DESC + H), (R - A, DESC + H + 36)])
        g.poly([(R + A, DESC + H), (R + A + 26, DESC + H), (R + A, DESC + H + 36)])

@glyph('a')
def _(g):
    if g.ital:
        cx, rx = 240, 200
        g.s(arc(cx, CY, rx, RY, 0, 360, 2.12))
        g.stem(cx + rx, 100, XH); g.tail(cx + rx)
        g.rect(cx + rx - 3, XH - H, cx + rx + 66, XH)
        return
    R = 400
    g.s(cat(line((R, 252), (250, 252)), arc(250, 127, 150, 125, 90, 332, 2.1)), N, 1.0, 0.9)
    g.s(cat(bez((96, 386), (120, 452), (180, 478), (250, 478)), bez((250, 478), (340, 478), (R, 430), (R, 330))))
    g.dot(92, 382, 37)
    g.stem(R, 0, 330); g.foot(R, 24, 88)

@glyph('g')
def _(g):
    cx, rx = 250, 205
    R = cx + rx
    g.s(arc(cx, CY, rx, RY, 0, 360, 2.12))
    g.stem(R, -62, XH)
    g.s(arc(cx, -62, rx, 146, 0, -166, 2.0), N, 1.0, 0.8)
    g.dot(cx + rx * math.cos(math.radians(-166)), -62 + 146 * math.sin(math.radians(-166)) + 22, 38)
    g.rect(R - 66, XH - H, R + 40, XH) if not g.ital else None

@glyph('j')
def _(g):
    L = 200
    g.stem(L, -90, XH); g.flag(L, XH)
    g.s(cat(line((L, -90), (L, -90)), bez((L, -90), (L, -176), (150, -208), (92, -208))), N, 1.0, 0.8)
    g.dot(86, -176, 38)
    g.dot(L + (6 if g.ital else 0), 628, 48)

@glyph('t')
def _(g):
    L = 170
    g.stem(L, 160, 640)
    g.s(cat(bez((L, 170), (L, 40), (230, 11), (290, 11)), bez((290, 11), (330, 11), (362, 28), (390, 66))), N, 1.0, 0.35)
    g.s(line((46, XH - 11), (318, XH - 11)), HR)
    g.rect(L - A, XH - 11 - 0, L + A, XH)  # keep bar solid through the stem

@glyph('f')
def _(g):
    L = 170
    top_ = 732
    g.stem(L, -180 if g.ital else 0, 560)
    g.s(bez((L, 560), (L, 676), (236, top_), (318, top_)), N, 1.0, 0.5)
    g.dot(338, 704, 40)
    g.s(line((46, XH - 11), (322, XH - 11)), HR)
    if g.ital:
        g.s(bez((L, -150), (L, -212), (120, -236), (56, -204)), N, 1.0, 0.45)
    else:
        g.foot(L)

@glyph('v')
def _(g):
    g.s(line((24, XH - 14), (232, 16)), HV)
    g.s(line((232, 16), (440, XH - 11)), HR)
    if not g.ital:
        g.top(24, XH, 62, 62); g.top(440, XH, 40, 62)

@glyph('w')
def _(g):
    g.s(line((20, XH - 14), (140, 16)), HV); g.s(line((140, 16), (260, 300)), HR)
    g.s(line((260, 300), (380, 16)), HV); g.s(line((380, 16), (500, XH - 11)), HR)
    g.top(20, XH, 40, 40); g.top(500, XH, 30, 40)

@glyph('x')
def _(g):
    g.s(line((36, XH - 14), (424, 14)), HV)
    g.s(line((36, 11), (424, XH - 11)), HR)
    if not g.ital:
        g.top(36, XH, 62, 62); g.top(424, XH, 62, 62); g.foot(36, 62, 62); g.foot(424, 62, 62)

@glyph('y')
def _(g):
    g.s(line((24, XH - 14), (212, 40)), HV)
    g.s(cat(line((440, XH - 11), (212, 30)), bez((212, 30), (150, -120), (130, -200), (62, -196))), N, 0.2, 0.9) if False else None
    g.s(line((440, XH - 11), (232, 30)), HR)
    g.s(bez((232, 30), (200, -80), (150, -190), (70, -196)), (30, 11), 1.0, 0.7)
    g.dot(64, -164, 38)
    if not g.ital:
        g.top(24, XH, 62, 62); g.top(440, XH, 40, 62)

@glyph('z')
def _(g):
    g.s(line((52, XH - 11), (400, XH - 11)), HR)
    g.s(line((52, 11), (420, 11)), HR)
    g.s(line((396, XH - 20), (60, 20)), MD)
    g.s(line((52, XH - 11), (52, XH - 80)), (13, 11))
    g.s(line((420, 11), (420, 80)), (13, 11))

@glyph('k')
def _(g):
    L = 60
    g.stem(L, 0, ASC); g.flag(L, ASC); g.foot(L)
    g.s(line((L + 10, 196), (384, XH - 11)), HR)
    g.s(line((204, 316), (420, 20)), HV)
    if not g.ital:
        g.top(384, XH, 20, 72); g.foot(420, 66, 66)

@glyph('s')
def _(g):
    p = cat(bez((372, 410), (340, 470), (300, 478), (240, 478)), bez((240, 478), (150, 478), (88, 440), (88, 372)),
            bez((88, 372), (88, 300), (160, 268), (235, 240)), bez((235, 240), (320, 208), (392, 180), (392, 110)),
            bez((392, 110), (392, 40), (330, 2), (240, 2)), bez((240, 2), (160, 2), (100, 30), (74, 100)))
    g.s(p, N, 1.0, 1.0)
    g.dot(374, 404, 36); g.dot(80, 96, 36)

# ───────────────────────── capitals ─────────────────────────

def slab(g, x0, x1, y, hang=True):
    g.rect(x0, y - H if hang else y, x1, y if hang else y + H)

def vtick(g, x, y0, y1): g.s(line((x, y0), (x, y1)), (12, 11))

@glyph('A')
def _(g):
    g.s(line((46, 14), (302, 690)), HR)
    g.s(line((302, 690), (580, 14)), HV)
    g.s(line((130, 214), (500, 214)), HR)
    g.foot(46, 74, 74); g.foot(580, 74, 74)

@glyph('B')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(cat(line((L, 689), (250, 689)), arc(250, 537, 172, 152, 90, -90, 2.05), line((250, 385), (L, 385))))
    g.s(cat(line((L, 385), (268, 385)), arc(268, 198, 196, 187, 90, -90, 2.05), line((268, 11), (L, 11))))
    g.top(L, CAP, 72, 0); g.foot(L, 72, 0)

@glyph('C')
def _(g):
    g.s(arc(340, 350, 296, 349, 40, 324, 2.1), N, 1.0, 0.85)
    g.dot(548, 614, 40)

@glyph('D')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(cat(line((L, 689), (270, 689)), arc(270, 350, 330, 339, 90, -90, 2.1), line((270, 11), (L, 11))))
    g.top(L, CAP, 72, 0); g.foot(L, 72, 0)

@glyph('E')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(line((L, 689), (440, 689)), HR); g.s(line((L, 360), (384, 360)), HR); g.s(line((L, 11), (480, 11)), HR)
    vtick(g, 440, 689, 596); vtick(g, 384, 360, 430); vtick(g, 384, 360, 296); vtick(g, 480, 11, 104)
    g.top(L, CAP, 72, 0); g.foot(L, 72, 0)

@glyph('F')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(line((L, 689), (440, 689)), HR); g.s(line((L, 360), (384, 360)), HR)
    vtick(g, 440, 689, 596); vtick(g, 384, 360, 430); vtick(g, 384, 360, 296)
    g.top(L, CAP, 72, 0); g.foot(L, 72, 72)

@glyph('G')
def _(g):
    g.s(arc(340, 350, 296, 349, 40, 336, 2.1), N, 1.0, 1.0)
    g.dot(548, 614, 40)
    g.s(line((408, 330), (626, 330)), HR)
    g.stem(616, 140, 330)

@glyph('H')
def _(g):
    g.stem(80, 0, CAP); g.stem(540, 0, CAP)
    g.s(line((80, 360), (540, 360)), HR)
    for x in (80, 540):
        g.top(x, CAP, 78, 78); g.foot(x, 78, 78)

@glyph('I')
def _(g):
    g.stem(160, 0, CAP); g.top(160, CAP, 150, 150); g.foot(160, 150, 150)

@glyph('J')
def _(g):
    g.stem(430, 200, CAP)
    g.s(bez((430, 220), (430, 60), (340, 11), (240, 11)), N, 1.0, 0.8)
    g.s(bez((240, 11), (170, 11), (110, 40), (92, 120)), N, 0.8, 0.5)
    g.dot(94, 130, 38)
    g.top(430, CAP, 104, 76)

@glyph('K')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(line((L + 10, 300), (546, 689)), HR)
    g.s(line((250, 440), (580, 16)), HV)
    g.top(L, CAP, 72, 0); g.foot(L, 72, 72); g.top(546, CAP, 20, 80); g.foot(580, 74, 74)

@glyph('L')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(line((L, 11), (470, 11)), HR); vtick(g, 470, 11, 120)
    g.top(L, CAP, 72, 0); g.foot(L, 72, 0)

@glyph('M')
def _(g):
    L, R = 70, 500
    g.stem(L, 0, CAP); g.stem(R, 0, CAP)
    g.s(line((L, 690), (285, 300)), HV); g.s(line((285, 300), (R, 690)), HR)
    g.top(L, CAP, 56, 0); g.foot(L, 56, 56); g.top(R, CAP, 0, 56); g.foot(R, 56, 56)

@glyph('N')
def _(g):
    g.stem(80, 0, CAP); g.stem(570, 0, CAP)
    g.s(line((80, 690), (570, 12)), HV)
    g.top(80, CAP, 78, 0); g.foot(80, 78, 78); g.top(570, CAP, 70, 78)

@glyph('O')
def _(g): g.s(arc(345, 350, 300, 349, 0, 360, 2.12))

@glyph('P')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(cat(line((L, 689), (280, 689)), arc(280, 482, 188, 207, 90, -90, 2.05), line((280, 275), (L, 275))))
    g.top(L, CAP, 72, 0); g.foot(L, 78, 78)

@glyph('Q')
def _(g):
    g.s(arc(345, 350, 300, 349, 0, 360, 2.12))
    g.s(bez((420, 120), (470, 60), (540, -60), (640, -110)), (28, 12), 1.0, 0.6)

@glyph('R')
def _(g):
    L = 80
    g.stem(L, 0, CAP)
    g.s(cat(line((L, 689), (270, 689)), arc(270, 490, 182, 199, 90, -90, 2.05), line((270, 291), (L, 291))))
    g.s(line((250, 296), (534, 14)), HV)
    g.top(L, CAP, 72, 0); g.foot(L, 78, 78); g.foot(534, 70, 76)

@glyph('S')
def _(g):
    p = cat(bez((566, 612), (520, 692), (450, 699), (350, 699)), bez((350, 699), (220, 699), (130, 650), (130, 545)),
            bez((130, 545), (130, 430), (240, 385), (345, 350)), bez((345, 350), (460, 312), (570, 270), (570, 160)),
            bez((570, 160), (570, 60), (490, 1), (350, 1)), bez((350, 1), (240, 1), (160, 40), (122, 134)))
    g.s(p); g.dot(566, 606, 38); g.dot(126, 130, 38)

@glyph('T')
def _(g):
    g.stem(270, 0, CAP)
    g.s(line((30, 689), (510, 689)), HR)
    vtick(g, 30, 689, 596); vtick(g, 510, 689, 596)
    g.foot(270, 84, 84)

@glyph('U')
def _(g):
    L, R = 80, 540
    g.stem(L, 220, CAP); g.stem(R, 220, CAP)
    g.s(arc(310, 220, 230, 209, 180, 360, 2.1))
    g.top(L, CAP, 78, 78); g.top(R, CAP, 78, 78)

@glyph('V')
def _(g):
    g.s(line((30, 686), (330, 16)), HV)
    g.s(line((330, 16), (632, 689)), HR)
    g.top(30, CAP, 72, 72); g.top(632, CAP, 40, 72)

@glyph('W')
def _(g):
    g.s(line((24, 686), (160, 16)), HV); g.s(line((160, 16), (290, 440)), HR)
    g.s(line((290, 440), (420, 16)), HV); g.s(line((420, 16), (556, 689)), HR)
    g.top(24, CAP, 50, 40); g.top(556, CAP, 40, 50)

@glyph('X')
def _(g):
    g.s(line((50, 686), (590, 14)), HV); g.s(line((50, 11), (590, 689)), HR)
    for x in (50, 590):
        g.top(x, CAP, 72, 72); g.foot(x, 72, 72)

@glyph('Y')
def _(g):
    g.s(line((30, 686), (330, 330)), HV); g.s(line((330, 330), (640, 689)), HR)
    g.stem(330, 0, 360); g.foot(330, 84, 84)
    g.top(30, CAP, 72, 72); g.top(640, CAP, 40, 72)

@glyph('Z')
def _(g):
    g.s(line((60, 689), (520, 689)), HR); g.s(line((60, 11), (570, 11)), HR)
    g.s(line((516, 676), (64, 24)), HV)
    vtick(g, 60, 689, 596); vtick(g, 570, 11, 108)

# ───────────────────────── figures ─────────────────────────

@glyph('0')
def _(g): g.s(arc(270, 345, 212, 340, 0, 360, 2.12)); g.s(line((150, 120), (390, 570)), HR)

@glyph('1')
def _(g):
    g.stem(270, 0, CAP); g.foot(270, 150, 150)
    g.s(line((270, 690), (130, 590)), (14, 11), 1.0, 1.0)

@glyph('2')
def _(g):
    p = cat(bez((84, 546), (100, 640), (170, 689), (260, 689)), bez((260, 689), (360, 689), (432, 640), (432, 550)),
            bez((432, 550), (432, 420), (230, 300), (74, 36)))
    g.s(p, N, 0.9, 1.0); g.dot(86, 540, 37)
    g.s(line((72, 11), (456, 11)), HR); vtick(g, 456, 11, 100)

@glyph('3')
def _(g):
    p = cat(bez((84, 570), (100, 650), (170, 689), (250, 689)), bez((250, 689), (350, 689), (410, 640), (410, 565)),
            bez((410, 565), (410, 480), (340, 410), (220, 392)), bez((220, 392), (350, 382), (460, 320), (460, 200)),
            bez((460, 200), (460, 70), (370, 11), (250, 11)), bez((250, 11), (160, 11), (95, 50), (72, 112)))
    g.s(p); g.dot(86, 562, 37); g.dot(76, 108, 37)

@glyph('4')
def _(g):
    g.stem(360, 0, CAP); g.foot(360, 90, 90)
    g.s(line((340, 690), (36, 214)), MD)
    g.s(line((36, 214), (484, 214)), HR)

@glyph('5')
def _(g):
    g.s(line((150, 689), (436, 689)), HR); vtick(g, 436, 689, 620)
    g.stem(150, 396, CAP)
    g.s(arc(255, 225, 202, 215, 115, -156, 2.05))
    g.dot(76, 134 + 12, 37)

@glyph('6')
def _(g):
    g.s(arc(252, 232, 198, 221, 0, 360, 2.1))
    g.s(bez((54, 240), (54, 500), (160, 690), (350, 690)), N, 1.0, 0.7)
    g.dot(372, 664, 38)

@glyph('7')
def _(g):
    g.s(line((50, 689), (476, 689)), HR); vtick(g, 50, 689, 600)
    g.s(bez((476, 689), (390, 480), (270, 250), (200, 16)), N, 0.9, 1.0)

@glyph('8')
def _(g):
    g.s(arc(250, 527, 152, 163, 0, 360, 2.1))
    g.s(arc(250, 192, 198, 181, 0, 360, 2.1))

@glyph('9')
def _(g):
    g.s(arc(252, 458, 198, 221, 0, 360, 2.1))
    g.s(bez((450, 450), (450, 190), (344, 0), (154, 0)), N, 1.0, 0.7)
    g.dot(132, 26, 38)

# ───────────────────────── punctuation ─────────────────────────

@glyph('.')
def _(g): g.dot(60, 52, 50)

@glyph(',')
def _(g):
    g.dot(60, 52, 50); g.s(bez((74, 40), (74, -10), (50, -60), (14, -92)), (36, 12), 1.0, 0.3)

@glyph(':')
def _(g): g.dot(60, 52, 50); g.dot(60, 410, 50)

@glyph(';')
def _(g):
    g.dot(60, 410, 50); g.dot(60, 52, 50); g.s(bez((74, 40), (74, -10), (50, -60), (14, -92)), (36, 12), 1.0, 0.3)

@glyph('!')
def _(g):
    g.s(line((60, 686), (60, 210)), (42, 12), 1.0, 0.4); g.dot(60, 52, 50)

@glyph('?')
def _(g):
    p = cat(bez((84, 556), (100, 644), (170, 689), (250, 689)), bez((250, 689), (350, 689), (410, 640), (410, 560)),
            bez((410, 560), (410, 470), (290, 440), (260, 360)), line((260, 360), (260, 250)))
    g.s(p, N, 1.0, 0.7); g.dot(86, 548, 37); g.dot(260, 52, 50)

@glyph("'")
def _(g): g.s(line((50, 690), (50, 540)), (30, 11), 1.0, 0.3)

@glyph('"')
def _(g):
    g.s(line((50, 690), (50, 540)), (30, 11), 1.0, 0.3); g.s(line((190, 690), (190, 540)), (30, 11), 1.0, 0.3)

@glyph('’')
def _(g):
    g.dot(60, 628, 44); g.s(bez((72, 618), (72, 580), (52, 548), (22, 520)), (34, 12), 1.0, 0.3)

@glyph('‘')
def _(g):
    g.dot(60, 628, 44); g.s(bez((48, 638), (48, 676), (68, 708), (98, 736)), (34, 12), 1.0, 0.3)

@glyph('“')
def _(g):
    for x in (60, 200):
        g.dot(x, 628, 44); g.s(bez((x - 12, 638), (x - 12, 676), (x + 8, 708), (x + 38, 736)), (34, 12), 1.0, 0.3)

@glyph('”')
def _(g):
    for x in (60, 200):
        g.dot(x, 628, 44); g.s(bez((x + 12, 618), (x + 12, 580), (x - 8, 548), (x - 38, 520)), (34, 12), 1.0, 0.3)

@glyph('(')
def _(g): g.s(arc(380, 240, 250, 480, 132, 228, 2.0), N, 1.0, 1.0, bulge=0.3)

@glyph(')')
def _(g): g.s(arc(-130, 240, 250, 480, 48, -48, 2.0), N, 1.0, 1.0, bulge=0.3)

@glyph('-')
def _(g): g.s(line((40, 250), (230, 250)), (14, 17))

@glyph('–')
def _(g): g.s(line((30, 250), (430, 250)), (14, 15))

@glyph('—')
def _(g): g.s(line((30, 250), (800, 250)), (14, 15))

@glyph('/')
def _(g): g.s(line((30, -120), (330, 720)), HR, 1.0, 1.0)

@glyph('+')
def _(g): g.s(line((40, 300), (400, 300)), (11, 13)); g.s(line((220, 120), (220, 480)), (13, 11))

@glyph('=')
def _(g): g.s(line((40, 380), (400, 380)), (11, 13)); g.s(line((40, 220), (400, 220)), (11, 13))

@glyph('·')
def _(g): g.dot(60, 270, 46)

@glyph('×')
def _(g): g.s(line((60, 120), (380, 440)), (30, 15)); g.s(line((60, 440), (380, 120)), (11, 11))

@glyph('°')
def _(g): g.s(arc(120, 560, 58, 58, 0, 360), (13, 13))

@glyph('&')
def _(g):
    g.s(arc(250, 540, 130, 150, 0, 360, 2.05), (22, 13))
    g.s(arc(255, 180, 200, 180, 0, 360, 2.05), (22, 13))
    g.s(line((120, 330), (440, 330)), (20, 13), 1.0, 1.0) if False else None
    g.s(line((470, 380), (560, 340)), (20, 13)) if False else None
    g.s(line((330, 400), (560, 10)), (26, 13))

@glyph('%')
def _(g):
    g.s(arc(120, 560, 100, 120, 0, 360, 2.0), (22, 13)); g.s(arc(440, 140, 100, 120, 0, 360, 2.0), (22, 13))
    g.s(line((500, 720), (60, -20)), (14, 14))

# ───────────────────────── terminal glyphs ─────────────────────────

@glyph('_')
def _(g): g.s(line((0, -70), (600, -70)), (14, 13))

@glyph('|')
def _(g): g.stem(60, -220, 760, (14, 11))

@glyph('<')
def _(g): g.s(cat(line((400, 520), (60, 300)), line((60, 300), (400, 80))), (14, 14))

@glyph('>')
def _(g): g.s(cat(line((60, 520), (400, 300)), line((400, 300), (60, 80))), (14, 14))

@glyph('\\')
def _(g): g.s(line((30, 720), (330, -120)), HR)

@glyph('`')
def _(g): g.s(line((40, 720), (150, 600)), (24, 11), 1.0, 0.4)

@glyph('^')
def _(g): g.s(cat(line((40, 400), (230, 690)), line((230, 690), (420, 400))), (11, 11))

@glyph('~')
def _(g): g.s(bez((30, 280), (110, 400), (190, 400), (250, 300)) + bez((250, 300), (310, 200), (390, 200), (470, 320))[1:], (14, 11))

@glyph('*')
def _(g):
    for a in (90, 18, -54, 162, 234):
        t = math.radians(a)
        g.s(line((220, 540), (220 + 150 * math.cos(t), 540 + 150 * math.sin(t))), (11, 11))
    g.dot(220, 540, 16)

@glyph('#')
def _(g):
    g.s(line((150, -40), (230, 690)), (14, 11)); g.s(line((330, -40), (410, 690)), (14, 11))
    g.s(line((60, 220), (500, 220)), HR); g.s(line((90, 460), (530, 460)), HR)

@glyph('$')
def _(g):
    p = cat(bez((380, 600), (340, 668), (290, 676), (240, 676)), bez((240, 676), (140, 676), (96, 620), (96, 548)),
            bez((96, 548), (96, 460), (170, 420), (250, 380)), bez((250, 380), (340, 336), (410, 300), (410, 200)),
            bez((410, 200), (410, 110), (350, 56), (250, 56)), bez((250, 56), (170, 56), (110, 90), (90, 160)))
    g.s(p); g.s(line((250, -80), (250, 760)), (11, 11))

@glyph('[')
def _(g):
    g.stem(150, -160, 760); g.s(line((150, 749), (330, 749)), HR); g.s(line((150, -149), (330, -149)), HR)

@glyph(']')
def _(g):
    g.stem(330, -160, 760); g.s(line((150, 749), (330, 749)), HR); g.s(line((150, -149), (330, -149)), HR)

@glyph('{')
def _(g):
    p = cat(bez((360, 760), (260, 760), (250, 700), (250, 620)), line((250, 620), (250, 400)),
            bez((250, 400), (250, 320), (200, 270), (110, 270)))
    g.s(p, N, 0.5, 0.5); g.s(cat(bez((110, 270), (200, 270), (250, 220), (250, 140)), line((250, 140), (250, -80)),
                                 bez((250, -80), (250, -150), (290, -170), (360, -170))), N, 0.5, 0.5)

@glyph('}')
def _(g):
    p = cat(bez((240, 760), (340, 760), (350, 700), (350, 620)), line((350, 620), (350, 400)),
            bez((350, 400), (350, 320), (400, 270), (490, 270)))
    g.s(p, N, 0.5, 0.5); g.s(cat(bez((490, 270), (400, 270), (350, 220), (350, 140)), line((350, 140), (350, -80)),
                                 bez((350, -80), (350, -150), (310, -170), (240, -170))), N, 0.5, 0.5)

@glyph('@')
def _(g):
    g.s(arc(290, 245, 275, 320, 20, 340, 2.0), (20, 13))
    g.s(arc(285, 250, 95, 125, 0, 360, 2.0), (18, 13))
    g.s(line((380, 360), (380, 150)), (18, 13))
    g.s(bez((380, 150), (380, 100), (500, 100), (500, 200)), (18, 13))

def arrow(g, ang):
    c = (300, 250); L = 250
    def P(d, w):
        t = math.radians(ang)
        return (c[0] + d * math.cos(t) - w * math.sin(t), c[1] + d * math.sin(t) + w * math.cos(t))
    g.s(line(P(-L, 0), P(L, 0)), (13, 13))
    g.s(cat(line(P(L - 150, 130), P(L, 0)), line(P(L, 0), P(L - 150, -130))), (13, 12))

@glyph('\u2192')
def _(g): arrow(g, 0)
@glyph('\u2190')
def _(g): arrow(g, 180)
@glyph('\u2191')
def _(g): arrow(g, 90)
@glyph('\u2193')
def _(g): arrow(g, -90)

@glyph('\u2588')
def _(g): g.rect(70, -140, 530, 760)

@glyph('\u258c')
def _(g): g.rect(70, -140, 300, 760)

# ───────────────────────── Cyrillic ─────────────────────────
#
# Same nib, serifs and pixel grid as the Latin. Capitals that share a Latin form (А В Е К М Н
# О Р С Т Х) reuse its drawing; the rest are built from the same parts. Most Cyrillic
# lowercase letters are small capitals in shape (в г д ж з и к л м н п т ц ч ш щ ъ ы ь э ю я),
# so each such letter is one function of its height: CAP for the capital, XH for the
# lowercase. Wide letters (Ж Ш Щ Ы Ю) are squeezed onto the mono cell like m and w.

CL, CR = 80, 540     # capital stems
WIDE_CYR = '\u0416\u0436\u0428\u0448\u0429\u0449\u042b\u044b\u042e\u044e\u043c\u0424\u0444\u2116'
LL, LR = 60, 420     # lowercase stems

def cyr(cap, low, L=None, R=None):
    """Registers f(g, h, L, R) as the capital (h = CAP) and, if given, the lowercase (h = XH)."""
    def deco(f):
        GL[cap] = lambda g: f(g, CAP, CL, CR)
        if low: GL[low] = lambda g: f(g, XH, LL, LR)
        return f
    return deco

def serif_w(h): return 72 if h == CAP else 62

def bar_top(g, x0, x1, h): g.s(line((x0, h - 11), (x1, h - 11)), HR)
def bar_base(g, x0, x1): g.s(line((x0, 11), (x1, 11)), HR)

def descender(g, x, depth=-150):
    """A short hairline drop below the baseline (Д Ц Щ)."""
    g.s(line((x, 11), (x, depth)), (14, 11))

def bowl_low(g, L, R, h, top=None):
    """Ь's bowl: from the stem at `top` round to the baseline (B's lower bowl at any height)."""
    yb = top if top is not None else round(h * 0.56)
    ry = (yb - 11) / 2
    cy = 11 + ry
    rx = min(ry * 1.08, (R - L) * 0.52)
    cx = R - rx - 10
    g.s(cat(line((L, yb), (cx, yb)), arc(cx, cy, rx, ry, 90, -90, 2.05), line((cx, 11), (L, 11))))

def breve(g, cx, y, w=120):
    g.s(arc(cx, y + 60, w, 64, 200, 340, 2.0), (16, 12), 1.0, 1.0)

def dieresis(g, cx, y, r=None):
    r = r or 44
    g.dot(cx - 110, y, r); g.dot(cx + 110, y, r)

@cyr('Б', 'б')            # Б б (lowercase б is its own form, overridden below)
def _(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); bar_top(g, L, R - 10, h); vtick(g, R - 10, h - 11, h - 96)
    bowl_low(g, L, R, h)
    g.top(L, h, w, 0); g.foot(L, w, 0)

@glyph('б')
def _(g):
    g.s(arc(250, CY, 205, RY, 0, 360, 2.12))
    g.s(bez((50, 250), (50, 560), (150, 680), (420, 712)), N, 1.0, 0.6)
    g.dot(432, 706, 36)

@glyph('в')
def _(g):
    L, h = LL, XH
    g.stem(L, 0, h)
    g.s(cat(line((L, h - 11), (230, h - 11)), arc(230, 368, 132, 101, 90, -90, 2.05), line((230, 267), (L, 267))))
    g.s(cat(line((L, 267), (250, 267)), arc(250, 139, 160, 128, 90, -90, 2.05), line((250, 11), (L, 11))))
    g.top(L, h, 62, 0); g.foot(L, 62, 0)

@cyr('Г', 'г')            # Г г
def _(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); bar_top(g, L, R - 40, h); vtick(g, R - 40, h - 11, h - 100)
    g.top(L, h, w, 0); g.foot(L, w, w)

@cyr('Д', 'д')            # Д д
def _(g, h, L, R):
    w = serif_w(h)
    l0, r0 = L - 40, R + 40
    g.s(bez((L + 10, 18), (L + 90, 160), (L + 120, h * 0.6), (L + 120, h - 11)), MD)
    bar_top(g, L + 120, R, h)
    g.stem(R, 0, h)
    bar_base(g, l0, r0)
    descender(g, l0, -150 if h == CAP else -140); descender(g, r0, -150 if h == CAP else -140)
    g.top(R, h, 0, w)

@cyr('Ж', 'ж')            # Ж ж
def _(g, h, L, R):
    w = serif_w(h)
    M = (L + R) / 2 + 10
    mid = h * 0.48
    g.stem(M, 0, h)
    g.s(line((M - 10, mid + 4), (L - 30, h - 11)), HR); g.s(line((M + 10, mid + 4), (R + 50, h - 11)), HR)
    g.s(line((M - 30, mid + 30), (L - 50, 16)), HV); g.s(line((M + 30, mid + 30), (R + 70, 16)), HV)
    g.top(M, h, w * 0.6, w * 0.6); g.foot(M, w * 0.6, w * 0.6)

def zeta(g, h, mirror=False):
    k = h / 700
    def P(x, y): return (x, y * k)
    p = cat(bez(P(84, 570), P(100, 650), P(170, 689), P(250, 689)), bez(P(250, 689), P(350, 689), P(410, 640), P(410, 565)),
            bez(P(410, 565), P(410, 480), P(340, 410), P(220, 392)), bez(P(220, 392), P(350, 382), P(460, 320), P(460, 200)),
            bez(P(460, 200), P(460, 70), P(370, 11), P(250, 11)), bez(P(250, 11), P(160, 11), P(95, 50), P(72, 112)))
    g.s(p); g.dot(86, 570 * k - 8, 37); g.dot(76, 112 * k - 4, 37)

@glyph('З')
def _(g): zeta(g, CAP)

@glyph('з')
def _(g): zeta(g, XH)

def ii(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); g.stem(R, 0, h)
    g.s(line((L, 14), (R, h - 14)), HV)
    g.top(L, h, w, w); g.top(R, h, w, w); g.foot(L, w, w); g.foot(R, w, w)

@cyr('И', 'и')            # И и
def _(g, h, L, R): ii(g, h, L, R)

@cyr('Й', 'й')            # Й й
def _(g, h, L, R):
    ii(g, h, L, R)
    breve(g, (L + R) / 2, h + 50 if h == CAP else 560)

def kk(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h)
    g.s(line((L + 10, h * 0.43), (R + 6, h - 11)), HR)
    g.s(line((L + h * 0.24, h * 0.62), (R + 40, 16)), HV)
    g.top(L, h, w, w); g.foot(L, w, w); g.top(R + 6, h, 20, w); g.foot(R + 40, w, w)

@glyph('К')
def _(g): GL['K'](g)

@glyph('к')
def _(g): kk(g, XH, LL, LR)

@cyr('Л', 'л')            # Л л
def _(g, h, L, R):
    w = serif_w(h)
    g.s(cat(bez((L - 50, 30), (L + 10, 40), (L + 90, 160), (L + 110, h - 11))), MD)
    g.dot(L - 44, 40, 36)
    bar_top(g, L + 110, R, h)
    g.stem(R, 0, h)
    g.top(R, h, 0, w); g.foot(R, w, w)

def mm(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); g.stem(R, 0, h)
    m = (L + R) / 2
    g.s(line((L, h - 10), (m, h * 0.42)), HV); g.s(line((m, h * 0.42), (R, h - 10)), HR)
    g.top(L, h, w * 0.8, 0); g.foot(L, w * 0.8, w * 0.8); g.top(R, h, 0, w * 0.8); g.foot(R, w * 0.8, w * 0.8)

@glyph('М')
def _(g): GL['M'](g)

@glyph('м')
def _(g): mm(g, XH, LL - 10, LR + 30)

def nn(g, h, L, R, bar=None):
    w = serif_w(h)
    g.stem(L, 0, h); g.stem(R, 0, h)
    g.s(line((L, bar or h * 0.51), (R, bar or h * 0.51)), HR)
    for x in (L, R):
        g.top(x, h, w, w); g.foot(x, w, w)

@glyph('Н')
def _(g): GL['H'](g)

@glyph('н')
def _(g): nn(g, XH, LL, LR)

@cyr('П', 'п')            # П п
def _(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); g.stem(R, 0, h); bar_top(g, L, R, h)
    g.top(L, h, w, 0); g.top(R, h, 0, w); g.foot(L, w, w); g.foot(R, w, w)

@glyph('т')
def _(g):
    h = XH
    g.stem(240, 0, h)
    g.s(line((30, h - 11), (450, h - 11)), HR)
    vtick(g, 30, h - 11, h - 90); vtick(g, 450, h - 11, h - 90)
    g.foot(240, 72, 72)

@glyph('У')
def _(g):
    h = CAP
    g.s(line((30, h - 14), (330, 250)), HV)
    g.s(line((620, h - 11), (300, 120)), HR)
    g.s(bez((300, 120), (260, 30), (200, 11), (120, 11)), (30, 11), 1.0, 0.7)
    g.dot(112, 40, 38)
    g.top(30, h, 72, 72); g.top(620, h, 40, 72)

@cyr('Ф', None)                # Ф
def _(g, h, L, R):
    m = (L + R) / 2 + 20
    g.stem(m, 0, h)
    g.s(arc(m, h * 0.52, 270, h * 0.27, 0, 360, 2.1))
    g.top(m, h, 84, 84); g.foot(m, 84, 84)

@glyph('ф')
def _(g):
    m = 250
    g.stem(m, DESC, ASC)
    g.s(arc(m, CY, 210, RY - 10, 0, 360, 2.1))
    g.top(m, ASC, 62, 0)

@cyr('Ц', 'ц')            # Ц ц
def _(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); g.stem(R, 0, h)
    bar_base(g, L, R + 50); descender(g, R + 50, -150 if h == CAP else -140)
    g.top(L, h, w, w); g.top(R, h, w, w)

@cyr('Ч', 'ч')            # Ч ч
def _(g, h, L, R):
    w = serif_w(h)
    g.stem(R, 0, h)
    g.s(cat(line((L, h - 20), (L, h * 0.62)), bez((L, h * 0.62), (L, h * 0.4), (L + 90, h * 0.38), (R, h * 0.4))), N, 1.0, 0.8)
    g.top(L, h, w, w); g.top(R, h, w, w); g.foot(R, w, w)

def sha(g, h, L, R, tail=False):
    w = serif_w(h) * 0.7
    M = (L + R) / 2
    for x in (L, M, R):
        g.stem(x, 0, h); g.top(x, h, w, w)
    bar_base(g, L, R + (50 if tail else 0))
    if tail: descender(g, R + 50, -150 if h == CAP else -140)

@cyr('Ш', 'ш')            # Ш ш
def _(g, h, L, R): sha(g, h, L - 20, R + 30)

@cyr('Щ', 'щ')            # Щ щ
def _(g, h, L, R): sha(g, h, L - 20, R + 10, tail=True)

@cyr('Ъ', 'ъ')            # Ъ ъ
def _(g, h, L, R):
    S = L + 110
    g.stem(S, 0, h); g.s(line((L - 60, h - 11), (S, h - 11)), HR); vtick(g, L - 60, h - 11, h - 96)
    bowl_low(g, S, R + 30, h)

@cyr('Ы', 'ы')            # Ы ы
def _(g, h, L, R):
    w = serif_w(h) * 0.7
    g.stem(L - 20, 0, h); g.top(L - 20, h, w, w)
    bowl_low(g, L - 20, R - 70, h)
    g.stem(R + 30, 0, h); g.top(R + 30, h, w, w); g.foot(R + 30, w, w)

@cyr('Ь', 'ь')            # Ь ь
def _(g, h, L, R):
    w = serif_w(h)
    g.stem(L, 0, h); g.top(L, h, w, w)
    bowl_low(g, L, R, h)

@glyph('Э')
def _(g):
    g.s(arc(300, 350, 296, 349, 140, -144, 2.1), N, 1.0, 0.85)
    g.dot(92, 614, 40)
    g.s(line((220, 360), (590, 360)), HR)

@glyph('э')
def _(g):
    g.s(arc(250, CY, 208, RY, 140, -142, 2.12), N, 1.0, 0.85)
    g.dot(86, 392, 37)
    g.s(line((150, CY + 4), (458, CY + 4)), HR)

@cyr('Ю', 'ю')            # Ю ю
def _(g, h, L, R):
    w = serif_w(h) * 0.7
    L = L - 30
    g.stem(L, 0, h); g.top(L, h, w, w); g.foot(L, w, w)
    g.s(line((L, h * 0.5), (L + 110, h * 0.5)), HR)
    rx = (R + 40 - (L + 110)) / 2
    g.s(arc(L + 110 + rx, h / 2, rx, h / 2 - 1, 0, 360, 2.12))

@cyr('Я', 'я')            # Я я
def _(g, h, L, R):
    w = serif_w(h)
    yb = round(h * 0.42)
    ry = (h - 11 - yb) / 2
    cx = L + ry * 1.0 + 40
    g.stem(R, 0, h)
    g.s(cat(line((R, h - 11), (cx, h - 11)), arc(cx, yb + ry, ry * 1.0 + 30, ry, 90, 270, 2.05), line((cx, yb), (R, yb))))
    g.s(line((cx + 30, yb - 6), (L - 20, 16)), HV)
    g.top(R, h, 0, w); g.foot(R, w, w); g.foot(L - 20, w, w * 0.6)

@glyph('Ё')                     # Ё
def _(g):
    GL['E'](g); dieresis(g, 260, 800, 40)

@glyph('ё')                     # ё
def _(g):
    GL['e'](g); dieresis(g, 250, 628, 44)

for _cyr, _lat in zip('АВЕОРСТХаеорсух',
                      'ABEOPCTXaeopcyx'):
    GL[_cyr] = (lambda lat: lambda g: GL[lat](g))(_lat)

# ───────────────────────── Russian punctuation ─────────────────────────

def guillemet(g, x, right):
    s = 1 if right else -1
    g.s(cat(line((x - s * 70, 400), (x + s * 70, 250)), line((x + s * 70, 250), (x - s * 70, 100))), (14, 13))

@glyph('«')
def _(g): guillemet(g, 150, False); guillemet(g, 330, False)

@glyph('»')
def _(g): guillemet(g, 150, True); guillemet(g, 330, True)

@glyph('…')
def _(g):
    for x in (60, 250, 440): g.dot(x, 52, 46)

@glyph('−')
def _(g): g.s(line((40, 300), (400, 300)), (11, 13))

@glyph('№')
def _(g):
    g.stem(40, 0, CAP, (20, 11)); g.stem(330, 0, CAP, (20, 11))
    g.s(line((40, 690), (330, 12)), MD)
    g.s(arc(470, 520, 70, 90, 0, 360, 2.0), (14, 11))
    g.s(line((390, 330), (550, 330)), HR)

@glyph('⏎')
def _(g):
    g.s(cat(line((480, 560), (480, 260)), line((480, 260), (80, 260))), (14, 13))
    g.s(cat(line((200, 380), (80, 260)), line((80, 260), (200, 140))), (14, 12))

# ───────────────────────── assembly ─────────────────────────

ROUND = set('ocegdbpqOCGQ0689')
def spacing(ch, ital):
    l = r = 26 if ital else 34
    if ch in ROUND: l = r = 40 if not ital else 34
    if ch in 'vwxyzVWXYAKTZ': l = r = 14 if not ital else 12
    if ch in '.,:;!\'"’‘“”': l = r = 34
    if ch in '-–—': l = r = 40
    return l, r

def build_glyph(ch, ital):
    def make(xf):
        g = G(ital, xf)
        GL[ch](g)
        return g.geom()
    geom = make(1.0)
    if MONO:
        cell, d = 570, (BUF - 7 if ch in 'mwMW@&%' + WIDE_CYR else BUF)   # crowded glyphs get a lighter fatten so counters stay open
        limit = 570 if ch == '_' else 480 if ch in '\u2588\u258c' else 540 if ch in '@%&' else 512
        minx, _, maxx, _ = geom.bounds
        if maxx - minx + 2 * d > limit:     # squeeze the skeleton, not the outline, so stems keep their weight
            geom = make((limit - 2 * d) / (maxx - minx))
        geom = geom.buffer(d, resolution=8)
        geom = affinity.translate(geom, yoff=d)
        geom = affinity.scale(geom, xfact=1, yfact=XH / (XH + 2 * d), origin=(0, 0))
        minx, _, maxx, _ = geom.bounds
        # centre on the pixel grid so left/right stairs match
        xo = round(((cell - (maxx - minx)) / 2 - minx) / PIXEL) * PIXEL if PIXEL else (cell - (maxx - minx)) / 2 - minx
        geom = affinity.translate(geom, xoff=xo)
        if PIXEL: geom = pixelate(geom)
        return geom, cell, int(round(geom.bounds[0]))
    if ital:
        geom = affinity.scale(geom, xfact=NARROW, yfact=1, origin=(0, 0))
        geom = affinity.skew(geom, xs=SLANT, origin=(0, 0))
    minx, _, maxx, _ = geom.bounds
    l, r = spacing(ch, ital)
    geom = affinity.translate(geom, xoff=l - minx)
    return geom, int(round(maxx - minx + l + r)), l

def pixelate(geom, px=PIXEL, cover=PIXEL_COVER):
    """Snap the smooth outline onto a coarse bitmap grid: stair-stepped curves and diagonals, square corners."""
    from shapely.geometry import box
    from shapely.prepared import prep
    minx, miny, maxx, maxy = geom.bounds
    pg = prep(geom)
    cells = []
    for i in range(int(math.floor(minx / px)), int(math.ceil(maxx / px))):
        for j in range(int(math.floor(miny / px)), int(math.ceil(maxy / px))):
            b = box(i * px, j * px, (i + 1) * px, (j + 1) * px)
            if pg.intersects(b) and geom.intersection(b).area >= cover * px * px:
                cells.append(b)
    return unary_union(cells).buffer(0)

def to_glyph(geom):
    pen = TTGlyphPen(None)
    polys = [geom] if geom.geom_type == 'Polygon' else list(geom.geoms)
    for poly in polys:
        poly = orient(poly.simplify(0.35 if not PIXEL else 0.01), sign=-1.0)
        for ring in [poly.exterior, *poly.interiors]:
            c = [(int(round(x)), int(round(y))) for x, y in ring.coords[:-1]]
            dd = [c[0]] + [p for p, q in zip(c[1:], c) if p != q]
            if len(dd) < 3: continue
            pen.moveTo(dd[0])
            for p in dd[1:]: pen.lineTo(p)
            pen.closePath()
    return pen.glyph()

def uni_name(ch):
    return {' ': 'space', '.': 'period', ',': 'comma', ':': 'colon', ';': 'semicolon', '!': 'exclam', '?': 'question',
            "'": 'quotesingle', '"': 'quotedbl', '(': 'parenleft', ')': 'parenright', '-': 'hyphen', '/': 'slash',
            '+': 'plus', '=': 'equal', '&': 'ampersand', '@': 'at', '%': 'percent', '’': 'quoteright',
            '‘': 'quoteleft', '“': 'quotedblleft', '”': 'quotedblright', '–': 'endash',
            '—': 'emdash', '_': 'underscore', '|': 'bar', '<': 'less', '>': 'greater', '\\': 'backslash', '`': 'grave', '^': 'asciicircum', '~': 'asciitilde', '*': 'asterisk', '#': 'numbersign', '$': 'dollar', '[': 'bracketleft', ']': 'bracketright', '{': 'braceleft', '}': 'braceright', '\u2192': 'arrowright', '\u2190': 'arrowleft', '\u2191': 'arrowup', '\u2193': 'arrowdown', '\u2588': 'fullblock', '\u258c': 'leftblock',  '·': 'periodcentered', '×': 'multiply', '°': 'degree',
            **{str(i): n for i, n in enumerate('zero one two three four five six seven eight nine'.split())}}.get(
                ch, ch if ch.isascii() else f'uni{ord(ch):04X}')

def build(ital, out):
    order, cmap, metrics, glyphs = ['.notdef'], {}, {}, {}
    nd = G(ital); nd.rect(60, 0, 540, 700); nd.parts[0] = nd.parts[0].difference(Polygon([(120, 60), (480, 60), (480, 640), (120, 640)]))
    glyphs['.notdef'] = to_glyph(nd.geom()); metrics['.notdef'] = (600, 60)
    glyphs['space'] = TTGlyphPen(None).glyph(); metrics['space'] = (230, 0); order.append('space'); cmap[32] = 'space'
    for ch in GL:
        name = uni_name(ch)
        geom, adv, lsb = build_glyph(ch, ital)
        glyphs[name] = to_glyph(geom); metrics[name] = (adv, lsb); order.append(name); cmap[ord(ch)] = name
        print('.', end='', flush=True)
    fam = 'Coppice Mono' if MONO else 'Coppice'; sty = 'Italic' if ital else 'Regular'
    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order); fb.setupCharacterMap(cmap); fb.setupGlyf(glyphs); fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=800, descent=-260)
    fb.setupNameTable({'familyName': fam, 'styleName': sty, 'uniqueFontIdentifier': f'{fam}-{sty}', 'fullName': f'{fam} {sty}',
                       'psName': f"{fam.replace(' ', '')}-{sty}", 'version': 'Version 1.000'})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-260, sTypoLineGap=0, usWinAscent=960, usWinDescent=300,
                sxHeight=XH, sCapHeight=CAP, fsType=0, achVendID='ASMR',
                fsSelection=(0x01 if ital else 0x40) | 0x80, usWeightClass=400, version=4)
    fb.setupPost(italicAngle=-SLANT if ital else 0, isFixedPitch=1 if MONO else 0)
    fb.font['head'].macStyle = 2 if ital else 0
    fb.save(str(out))
    print(f'\n{out}')

if __name__ == '__main__':
    outdir = Path(sys.argv[1] if len(sys.argv) > 1 else '.')
    outdir.mkdir(parents=True, exist_ok=True)
    # One upright style only: the app should never mix roman and italic.
    build(False, outdir / 'CoppiceMono-Regular.ttf')
