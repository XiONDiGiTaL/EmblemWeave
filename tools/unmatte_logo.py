"""Cut a logo out of a white background, keeping soft edges and glows.

Each pixel is treated as the logo composited over white and solved for its
true colour and opacity ("colour to alpha"): a cyan glow fading into white
becomes cyan fading to transparent, so it sits correctly on a dark theme.
JPEG noise in the background (pixels within `--floor` of white) becomes fully
transparent, and the result is trimmed to the logo with a small margin.

usage: python tools/unmatte_logo.py INPUT OUTPUT [--floor 10] [--margin 8]
"""
import argparse

import numpy as np
from PIL import Image


def unmatte(rgb: np.ndarray, floor: float) -> np.ndarray:
    c = rgb.astype(np.float64) / 255.0
    deficit = 1.0 - c  # how far each channel is from white
    alpha = deficit.max(axis=2)
    # Background noise from compression: anything this close to white is white.
    f = floor / 255.0
    alpha = np.clip((alpha - f) / (1.0 - f), 0.0, 1.0)
    safe = np.where(alpha > 0, alpha, 1.0)[..., None]
    # c = a * colour + (1 - a) * white  =>  colour = (c - (1 - a)) / a
    colour = np.clip((c - (1.0 - safe)) / safe, 0.0, 1.0)
    out = np.dstack([colour, alpha])
    return (out * 255.0 + 0.5).astype(np.uint8)


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("input")
    p.add_argument("output")
    p.add_argument("--floor", type=float, default=10.0)
    p.add_argument("--margin", type=int, default=8)
    a = p.parse_args()
    rgba = unmatte(np.asarray(Image.open(a.input).convert("RGB")), a.floor)
    ys, xs = np.nonzero(rgba[..., 3] > 8)
    y0, y1 = max(ys.min() - a.margin, 0), min(ys.max() + a.margin + 1, rgba.shape[0])
    x0, x1 = max(xs.min() - a.margin, 0), min(xs.max() + a.margin + 1, rgba.shape[1])
    Image.fromarray(rgba[y0:y1, x0:x1], "RGBA").save(a.output, optimize=True)
    print(f"{a.output}: {x1 - x0}x{y1 - y0}")


if __name__ == "__main__":
    main()
