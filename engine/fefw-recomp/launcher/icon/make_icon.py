# SPDX-License-Identifier: GPL-3.0-or-later
"""Draws the project's app icon: an "FW" monogram in gold on a deep indigo tile. No outside artwork is used; the letters are set in a
serif font installed with Windows (rasterised, as the font licences allow). Needs Pillow.

    python launcher/icon/make_icon.py [--font constanb.ttf] [--out-png PATH]

Writes launcher/icon/icon.png (1024 px, the source for `npx tauri icon`) and game-build/fefw/res/fefw.ico (16 to 256 px).
"""
import argparse
import math
import os

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
S = 2048
FONTS = ['constanb.ttf', 'palab.ttf', 'georgiab.ttf', 'timesbd.ttf']


def radial(size, inner, outer, center=(0.5, 0.38), radius=0.85):
    """A radial gradient from inner (at center) to outer."""
    small = 256
    g = Image.new('RGB', (small, small))
    px = g.load()
    cx, cy = center[0] * small, center[1] * small
    for y in range(small):
        for x in range(small):
            t = min(1.0, math.hypot(x - cx, y - cy) / (radius * small))
            t = t * t * (3 - 2 * t)
            px[x, y] = tuple(int(inner[i] + (outer[i] - inner[i]) * t) for i in range(3))
    return g.resize((size, size), Image.BICUBIC)


def vertical(size, top, bottom):
    g = Image.new('RGB', (1, 256))
    for y in range(256):
        t = y / 255
        g.putpixel((0, y), tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)))
    return g.resize((size, size), Image.BICUBIC)


def draw_icon(font_path):
    radius = int(S * 0.225)
    tile_mask = Image.new('L', (S, S), 0)
    ImageDraw.Draw(tile_mask).rounded_rectangle((0, 0, S - 1, S - 1), radius=radius, fill=255)
    img = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    img.paste(radial(S, (58, 44, 112), (14, 12, 32)), (0, 0), tile_mask)

    # a fine gold inner frame
    frame = Image.new('L', (S, S), 0)
    d = ImageDraw.Draw(frame)
    inset = int(S * 0.055)
    d.rounded_rectangle((inset, inset, S - 1 - inset, S - 1 - inset), radius=radius - inset, outline=255, width=max(2, S // 170))
    gold_flat = Image.new('RGBA', (S, S), (214, 170, 92, 255))
    img.paste(gold_flat, (0, 0), frame.point(lambda v: v * 150 // 255))

    # the monogram: F and W overlapping a little, the W's first stroke tucked under the F
    font = ImageFont.truetype(font_path, int(S * 0.50))
    mono = Image.new('L', (S, S), 0)
    md = ImageDraw.Draw(mono)
    fb = md.textbbox((0, 0), 'F', font=font)
    wb = md.textbbox((0, 0), 'W', font=font)
    overlap = int((fb[2] - fb[0]) * 0.22)
    total = (fb[2] - fb[0]) + (wb[2] - wb[0]) - overlap
    x0 = (S - total) // 2
    h = max(fb[3], wb[3]) - min(fb[1], wb[1])
    y0 = (S - h) // 2 - min(fb[1], wb[1]) + int(S * 0.01)
    w_layer = Image.new('L', (S, S), 0)
    ImageDraw.Draw(w_layer).text((x0 + (fb[2] - fb[0]) - overlap - wb[0], y0), 'W', font=font, fill=255)
    f_layer = Image.new('L', (S, S), 0)
    ImageDraw.Draw(f_layer).text((x0 - fb[0], y0), 'F', font=font, fill=255)
    # a dark gap where the F crosses the W, so both letters read
    gap = f_layer.filter(ImageFilter.MaxFilter(size=(S // 110) | 1))
    w_cut = ImageChops.subtract(w_layer, gap)
    mono = ImageChops.lighter(w_cut, f_layer)

    # shadow, then gold with a light top edge
    shadow = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    shadow.putalpha(mono.filter(ImageFilter.GaussianBlur(S * 0.012)).point(lambda v: v * 170 // 255))
    img = Image.alpha_composite(img, ImageChops.offset(shadow, 0, int(S * 0.012)))
    gold = vertical(S, (255, 233, 168), (190, 128, 44)).convert('RGBA')
    gold.putalpha(mono)
    img = Image.alpha_composite(img, gold)
    edge = ImageChops.subtract(mono, ImageChops.offset(mono, 0, int(S * 0.006)))
    hi = Image.new('RGBA', (S, S), (255, 248, 220, 0))
    hi.putalpha(edge.point(lambda v: v * 120 // 255))
    img = Image.alpha_composite(img, hi)

    out = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    out.paste(img, (0, 0), tile_mask)
    return out


def find_font(name):
    if name and os.path.isfile(name):
        return name
    windir = os.environ.get('WINDIR', 'C:\\Windows')
    for f in ([name] if name else []) + FONTS:
        p = os.path.join(windir, 'Fonts', f)
        if os.path.isfile(p):
            return p
    raise SystemExit('no serif font found: pass --font <a .ttf>')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--font', default='')
    ap.add_argument('--out-png', default=os.path.join(HERE, 'icon.png'))
    ap.add_argument('--no-ico', action='store_true')
    a = ap.parse_args()
    big = draw_icon(find_font(a.font))
    big.resize((1024, 1024), Image.LANCZOS).save(a.out_png)
    if not a.no_ico:
        ico = os.path.join(ROOT, 'game-build', 'fefw', 'res', 'fefw.ico')
        os.makedirs(os.path.dirname(ico), exist_ok=True)
        big.resize((256, 256), Image.LANCZOS).save(ico, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print('wrote', a.out_png)


if __name__ == '__main__':
    main()
