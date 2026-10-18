# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow", "numpy"]
# ///
"""ASMR X3000 app icon: the wordmark ASMR / X / 3000 in Coppice Italic.
Run:  uv run tools/icon/make_icon.py <out.png>     (1024x1024, transparent margin, macOS grid)
"""
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageChops

S = 2                      # supersample
W = 1024 * S
BODY = 824 * S             # macOS icon body
C = W // 2

def blur(im, r): return im.filter(ImageFilter.GaussianBlur(r * S))

def squircle_mask(size, n=5.0):
    y, x = np.mgrid[0:W, 0:W].astype(np.float32)
    u, v = np.abs(x + .5 - C) / (size / 2), np.abs(y + .5 - C) / (size / 2)
    d = (u ** n + v ** n) ** (1 / n)
    return Image.fromarray((np.clip((1 - d) * size / 2 / 1.6 + .5, 0, 1) * 255).astype(np.uint8), 'L')

body = squircle_mask(BODY)

# ── background: deep graphite with a soft warm well of light behind the tree
y, x = np.mgrid[0:W, 0:W].astype(np.float32)
cx, cy = C, C - 30 * S
r = np.hypot(x - cx, (y - cy) * 1.05) / (BODY * .62)
well = np.clip(1 - r, 0, 1) ** 1.8
vert = (y - (C - BODY / 2)) / BODY                                   # 0 top → 1 bottom
base = 9 + 10 * (1 - vert)
bg = np.stack([base + 34 * well, base + 36 * well, base + 33 * well], -1)
img = Image.fromarray(np.clip(bg, 0, 255).astype(np.uint8), 'RGB').convert('RGBA')

# ── typographic lockup: ASMR / X ───── / 3000, set in Coppice Italic
from PIL import ImageFont
FONT = str(Path(__file__).resolve().parents[2] / 'apps/macos/Resources/Fonts/Coppice-Italic.ttf')
INK = (244, 240, 230)
L, R = C - 330 * S, C + 330 * S          # text block edges
def fit(text, width, track=0):
    """Largest font size whose ink width == width; returns (font, ink bbox)."""
    size = 400 * S
    f = ImageFont.truetype(FONT, size)
    bb = f.getbbox(text)
    size = int(size * width / (bb[2] - bb[0]))
    f = ImageFont.truetype(FONT, size)
    return f, f.getbbox(text)

layer = Image.new('L', (W, W), 0)
ld = ImageDraw.Draw(layer)
f1, b1 = fit('ASMR', R - L)
f3, b3 = fit('3000', R - L - 10 * S)
h1, h3 = b1[3] - b1[1], b3[3] - b3[1]
rule_h = 92 * S
gap = 30 * S
total = h1 + gap + rule_h + gap + h3
y = C - total // 2 + 6 * S
ld.text((L - b1[0], y - b1[1]), 'ASMR', font=f1, fill=255)
ry = y + h1 + gap
fx = ImageFont.truetype(FONT, int(rule_h * 1.75))
bx = fx.getbbox('X')
ld.text((L - bx[0], ry + rule_h // 2 - (bx[1] + bx[3]) // 2), 'X', font=fx, fill=255)
xr = L + (bx[2] - bx[0]) + 36 * S
ld.rectangle([xr, ry + rule_h // 2 - 2 * S, R, ry + rule_h // 2 + 2 * S], fill=255)
ld.rectangle([R - 2 * S, ry + 8 * S, R, ry + rule_h - 8 * S], fill=255)      # end tick
ty = ry + rule_h + gap
ld.text((L - b3[0] + 4 * S, ty - b3[1]), '3000', font=f3, fill=255)
mask = layer
glow = blur(mask, 26).point(lambda v: int(v * .35))
glow_rgb = Image.new('RGBA', (W, W), (255, 240, 215, 0)); glow_rgb.putalpha(glow)
img = Image.alpha_composite(img, glow_rgb)
fill = Image.new('RGBA', (W, W), INK + (255,)); fill.putalpha(mask)
img = Image.alpha_composite(img, fill)
tree_h = top = 0

# ── film grain (engraved paper feel)
rng = np.random.default_rng(3)
g = rng.normal(0, 3.2, (W, W, 1)).astype(np.float32)
arr = np.asarray(img).astype(np.float32); arr[..., :3] = np.clip(arr[..., :3] + g, 0, 255)
img = Image.fromarray(arr.astype(np.uint8), 'RGBA')

# ── clip to squircle, add rim light and drop shadow
img.putalpha(ImageChops.multiply(img.getchannel('A'), body))
rim_top = ImageChops.subtract(body, body.transform(body.size, Image.AFFINE, (1, 0, 0, 0, 1, -5 * S)))
rim = Image.new('RGBA', (W, W), (255, 255, 255, 0)); rim.putalpha(blur(rim_top, .8).point(lambda v: int(v * .32)))
rim_bot = ImageChops.subtract(body, body.transform(body.size, Image.AFFINE, (1, 0, 0, 0, 1, 5 * S)))
rimb = Image.new('RGBA', (W, W), (0, 0, 0, 0)); rimb.putalpha(blur(rim_bot, .8).point(lambda v: int(v * .5)))
img = Image.alpha_composite(Image.alpha_composite(img, rimb), rim)

drop = Image.new('RGBA', (W, W), (0, 0, 0, 0))
drop.putalpha(blur(body.transform(body.size, Image.AFFINE, (1, 0, 0, 0, 1, -12 * S)), 14).point(lambda v: int(v * .55)))
out = Image.alpha_composite(drop, img)
out.resize((1024, 1024), Image.LANCZOS).save(sys.argv[1])
