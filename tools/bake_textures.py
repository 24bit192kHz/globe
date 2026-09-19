#!/usr/bin/env python3
"""Build high-res ASCII earth textures from real imagery.

Day: luminance of blue-marble (ocean dark, land/ice bright) mapped to palette.
Night: night-lights brightness mapped to palette. Both quantized with
Floyd-Steinberg dithering so coastlines stay crisp instead of blocky.

Usage: python3 bake_textures.py [--cols N] [--rows N]
Writes ../globe/textures/earth_hd.txt and earth_night_hd.txt
"""
import sys
from PIL import Image
import numpy as np

PALETTE = list(" .:',;,wiogOLXHWYV@")
NIGHT_PALETTE = list(" .:;,.wiogOLXHWYV@")


def luminance(im):
    a = np.asarray(im.convert("RGB"), dtype=np.float32) / 255.0
    # day: land bright, ocean dark. green/red weighted, blue suppressed
    # (ocean is blue -> dark; land/ice -> bright)
    return (0.55 * a[..., 0] + 0.55 * a[..., 1] - 0.35 * a[..., 2]).clip(0, 1)


def night_lum(im):
    a = np.asarray(im.convert("RGB"), dtype=np.float32) / 255.0
    return (0.299 * a[..., 0] + 0.587 * a[..., 1] + 0.114 * a[..., 2]).clip(0, 1)


def dither(field, levels):
    """Floyd-Steinberg quantize field in [0,1] to levels indices."""
    h, w = field.shape
    buf = field.copy() * (levels - 1)
    out = np.zeros((h, w), dtype=np.uint8)
    for y in range(h):
        for x in range(w):
            old = buf[y, x]
            new = int(round(old))
            new = max(0, min(levels - 1, new))
            out[y, x] = new
            err = old - new
            if x + 1 < w:
                buf[y, x + 1] += err * 7 / 16
            if y + 1 < h:
                if x > 0:
                    buf[y + 1, x - 1] += err * 3 / 16
                buf[y + 1, x] += err * 5 / 16
                if x + 1 < w:
                    buf[y + 1, x + 1] += err * 1 / 16
    return out


def bake(src, palette, cols, rows, out, fn, gamma=1.0):
    im = Image.open(src)
    im = im.resize((cols, rows), Image.LANCZOS)
    lum = fn(im) ** gamma
    idx = dither(lum, len(palette))
    lines = ["".join(palette[i] for i in row) for row in idx]
    open(out, "w").write("\n".join(lines) + "\n")
    print(f"wrote {out} {cols}x{rows}")


def main():
    cols = int(sys.argv[sys.argv.index("--cols") + 1]) if "--cols" in sys.argv else 1440
    rows = int(sys.argv[sys.argv.index("--rows") + 1]) if "--rows" in sys.argv else 720
    bake("earth-day.jpg", PALETTE, cols, rows, "../globe/textures/earth_hd.txt", luminance, gamma=0.8)
    # night: gamma lifts dim city glow so coasts read against black ocean
    bake("earth-night.jpg", PALETTE, cols, rows, "../globe/textures/earth_night_hd.txt", night_lum, gamma=0.6)


if __name__ == "__main__":
    main()
