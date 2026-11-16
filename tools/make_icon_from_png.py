# Builds the Windows program icon from a supplied PNG.
#
# The launcher, the built fefw.exe and the taskbar read apps/emblemweave/assets/icon.ico.
# Nothing is drawn here: the artwork is taken as given, trimmed to its visible edge,
# centred on a transparent square and written at every size Windows asks for.
#
#   python tools/make_icon_from_png.py
#   python tools/make_icon_from_png.py --source assets/branding/icon-source.png

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--source", default="assets/branding/icon-source.png")
    ap.add_argument("--ico", default="apps/emblemweave/assets/icon.ico")
    ap.add_argument("--png", default="apps/emblemweave/assets/icon.png")
    args = ap.parse_args()

    source = Path(args.source)
    if not source.is_absolute():
        source = ROOT / source
    image = Image.open(source).convert("RGBA")

    # The visible box: alpha for a transparent file, otherwise anything that is not near-white.
    alpha = image.getchannel("A")
    box = alpha.getbbox()
    if box is None:
        mask = image.convert("RGB").point(lambda v: 255 if v < 244 else 0)
        box = mask.getbbox()
    if box is None:
        raise SystemExit(f"{source} has no visible artwork")

    left, top, right, bottom = box
    width, height = right - left, bottom - top
    # Square crop with a 5% margin, centred on the artwork, clamped to the image.
    side = int(max(width, height) * 1.10)
    cx, cy = (left + right) // 2, (top + bottom) // 2
    sx = max(0, min(cx - side // 2, image.width - side))
    sy = max(0, min(cy - side // 2, image.height - side))
    side = min(side, image.width - sx, image.height - sy)
    square = image.crop((sx, sy, sx + side, sy + side))

    # 256 px is the largest size an .ico holds; drawing at 512 first keeps the small sizes clean.
    big = square.resize((512, 512), Image.LANCZOS)
    canvas = Image.new("RGBA", (512, 512), (0, 0, 0, 0))
    canvas.alpha_composite(big)

    ico = ROOT / args.ico
    png = ROOT / args.png
    ico.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(ico, format="ICO", sizes=[(s, s) for s in SIZES])
    canvas.resize((256, 256), Image.LANCZOS).save(png, format="PNG")

    print(f"wrote {ico.relative_to(ROOT)} ({len(SIZES)} sizes) and {png.relative_to(ROOT)} from {source.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
