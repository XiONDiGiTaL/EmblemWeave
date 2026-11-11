"""Cut the EmblemWeave logo render out of its white background.

1. The outer background is a flood fill from the image border over light,
   low-saturation pixels.
2. The gaps enclosed inside and between letters are found separately, from
   pixels that are light and also smooth: background white is flat, while the
   silver letter faces are textured, so a face never joins the gap it touches.
   Enclosed regions that are compact, bright and flat become background; long
   thin runs (the silver rail along the bar) stay.
3. Around the background (grown a few pixels to take in antialiased edges),
   colour-to-alpha against white turns the soft edge into transparency, and
   only a narrow halo next to the logo keeps any alpha, so the backdrop's faint
   grey gradient does not survive as a haze.

Usage: python tools/cutout_logo.py <render> <out.png>
"""
import sys
from collections import deque

import numpy as np
from PIL import Image, ImageFilter

src, out = sys.argv[1], sys.argv[2]
rgb = np.asarray(Image.open(src).convert("RGB")).astype(np.float32)
h, w, _ = rgb.shape
mx, mn = rgb.max(axis=2), rgb.min(axis=2)
light = (mn > 175) & ((mx - mn) < 34)


def box(a, r=2):
    """Mean over a (2r+1) square, edges replicated."""
    p = np.pad(a, r, mode="edge")
    c = np.cumsum(np.cumsum(p, axis=0), axis=1)
    c = np.pad(c, ((1, 0), (1, 0)))
    k = 2 * r + 1
    return (c[k:, k:] - c[:-k, k:] - c[k:, :-k] + c[:-k, :-k]) / (k * k)


lum = rgb.mean(axis=2)
flat = np.sqrt(np.maximum(box(lum * lum) - box(lum) ** 2, 0.0)) < 5.0


def regions(mask):
    """Connected regions of `mask`, as lists of (y, x)."""
    seen = np.zeros((h, w), bool)
    for sy in range(h):
        for sx in range(w):
            if not mask[sy, sx] or seen[sy, sx]:
                continue
            seen[sy, sx] = True
            pixels = [(sy, sx)]
            q = deque(pixels)
            while q:
                y, x = q.popleft()
                for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    ny, nx = y + dy, x + dx
                    if 0 <= ny < h and 0 <= nx < w and mask[ny, nx] and not seen[ny, nx]:
                        seen[ny, nx] = True
                        pixels.append((ny, nx))
                        q.append((ny, nx))
            yield pixels


bg = np.zeros((h, w), bool)
for pixels in regions(light):
    ys, xs = zip(*pixels)
    if min(ys) == 0 or min(xs) == 0 or max(ys) == h - 1 or max(xs) == w - 1:
        bg[ys, xs] = True

for pixels in regions(light & flat & ~bg):
    if len(pixels) <= 40:
        continue
    ys, xs = zip(*pixels)
    region = rgb[ys, xs]
    tall, wide = max(ys) - min(ys) + 1, max(xs) - min(xs) + 1
    compact = max(tall, wide) < 8 * min(tall, wide)
    if compact and region.min(axis=1).mean() > 195 and region.std(axis=0).mean() < 24:
        bg[ys, xs] = True


def grow(mask, times):
    img = Image.fromarray((mask * 255).astype(np.uint8))
    for _ in range(times):
        img = img.filter(ImageFilter.MaxFilter(3))
    return np.asarray(img) > 0


edge = grow(bg, 2)
halo = grow(~bg, 7)

# Colour-to-alpha against white.
a = np.clip((255.0 - mn) / 255.0 * 1.25, 0.0, 1.0)
safe = np.maximum(a, 1e-3)[..., None]
unmixed = np.clip((rgb - 255.0 * (1.0 - safe)) / safe, 0, 255)
alpha = np.where(edge, a, 1.0)
alpha = np.where(bg & ~halo, 0.0, alpha)
alpha = np.where(alpha < 0.05, 0.0, alpha)
colour = np.where(edge[..., None], unmixed, rgb)

rgba = np.dstack([colour, alpha * 255.0]).astype(np.uint8)
img = Image.fromarray(rgba, "RGBA")
img = img.crop(img.getbbox())
img.save(out)
print(out, img.size)
