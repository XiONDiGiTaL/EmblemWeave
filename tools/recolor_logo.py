"""Recolour the cut-out EmblemWeave logo, one colourway per scheme.

(The Arcade scheme has its own 8-bit artwork, cut out of its white background
by tools/unmatte_logo.py.)

The render has three materials: gold (EMBLEM and the scrollwork), silver
(WEAVE and the rims) and royal blue (the outline band, the bar and the glow).
Each pixel is classified by hue and saturation and moved to the scheme's
colour for that material. Only hue and saturation change; brightness is kept,
so the metal shading, bevels and highlights of the render survive.

Usage: python tools/recolor_logo.py assets/branding/logo-fe.png assets/branding
       python tools/recolor_logo.py assets/branding/logo-fe.png assets/branding ember frost
"""
import colorsys
import sys

import numpy as np
from PIL import Image

src, out_dir = sys.argv[1], sys.argv[2]
img = np.asarray(Image.open(src).convert("RGBA")).astype(np.float32) / 255.0
rgb, alpha = img[..., :3], img[..., 3]

to_hsv = np.vectorize(colorsys.rgb_to_hsv)
to_rgb = np.vectorize(colorsys.hsv_to_rgb)
hue, sat, val = to_hsv(rgb[..., 0], rgb[..., 1], rgb[..., 2])
deg = hue * 360.0

gold = (deg >= 18) & (deg <= 62) & (sat > 0.20)
blue = (deg >= 195) & (deg <= 265) & (sat > 0.25)
silver = ~gold & ~blue


def hsv_of(hex_colour):
    r, g, b = (int(hex_colour[i:i + 2], 16) / 255.0 for i in (1, 3, 5))
    return colorsys.rgb_to_hsv(r, g, b)


def recolour(name, gold_to, silver_to, blue_to, gold_sat, silver_sat, blue_sat, gold_val=1.0, silver_val=1.0, blue_val=1.0):
    h, s, v = hue.copy(), sat.copy(), val.copy()
    for mask, target, sat_scale, val_scale in (
        (gold, gold_to, gold_sat, gold_val),
        (silver, silver_to, silver_sat, silver_val),
        (blue, blue_to, blue_sat, blue_val),
    ):
        th, ts, _ = hsv_of(target)
        h = np.where(mask, th, h)
        # Saturation follows the target, scaled by how saturated the pixel
        # was within its material, so highlights stay pale.
        rel = np.clip(sat / max(float(sat[mask].mean()) if mask.any() else 1.0, 1e-3), 0.0, 1.6)
        s = np.where(mask, np.clip(ts * sat_scale * rel, 0.0, 1.0), s)
        v = np.where(mask, np.clip(v * val_scale, 0.0, 1.0), v)
    r, g, b = to_rgb(h, s, v)
    out = np.dstack([r, g, b, alpha]) * 255.0
    path = f"{out_dir}/logo-{name}.png"
    Image.fromarray(out.astype(np.uint8), "RGBA").save(path)
    print(path)


# One entry per scheme: the render's three materials mapped to the scheme's
# colours. Arcade has its own pixel-art logo, cut out by tools/unmatte_logo.py.
SCHEMES = {
    # xiondigital.net: silver-white faces, the site's blue band.
    "xion": dict(gold_to="#ecedef", silver_to="#c6cad0", blue_to="#3f80ff",
                 gold_sat=0.05, silver_sat=0.6, blue_sat=1.1,
                 gold_val=1.18, silver_val=1.05, blue_val=1.45),
    # Ember: forge orange, hot ash silver and oxblood steel.
    "ember": dict(gold_to="#f97316", silver_to="#e7c8bd", blue_to="#7a1512",
                  gold_sat=1.15, silver_sat=0.55, blue_sat=1.0,
                  gold_val=1.05, silver_val=1.02, blue_val=1.15),
    # Verdant: pale gold, jade silver and deep emerald.
    "verdant": dict(gold_to="#d9c56a", silver_to="#7fd8b0", blue_to="#0e6b4c",
                    gold_sat=0.95, silver_sat=0.5, blue_sat=1.0,
                    gold_val=1.0, silver_val=1.0, blue_val=1.05),
    # Frost: ice silver, snow white and glacier blue.
    "frost": dict(gold_to="#bfe9ff", silver_to="#eaf6ff", blue_to="#1d6fa8",
                  gold_sat=0.6, silver_sat=0.25, blue_sat=1.05,
                  gold_val=1.1, silver_val=1.12, blue_val=1.05),
}

for name in sys.argv[3:] or list(SCHEMES):
    recolour(name, **SCHEMES[name])
