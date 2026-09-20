#!/usr/bin/env python3
"""Build high-res ASCII earth textures from real imagery.

Day: luminance of blue-marble (ocean dark, land/ice bright) mapped to palette.
Night: night-lights brightness mapped to palette. Both quantized with
Floyd-Steinberg dithering so coastlines stay crisp instead of blocky.

Usage: python3 bake_textures.py [--cols N] [--rows N] [--text]
       python3 bake_textures.py --from-text DAY_TXT NIGHT_TXT [--text]
Writes ../globe/textures/earth_hd.gidx and earth_night_hd.gidx by default
(GIDX1 binary index format, 18-char library palette embedded).
With --text, also writes the legacy ../globe/textures/earth_hd.txt and
earth_night_hd.txt dumps. With --from-text, converts the given committed
.txt files to .gidx without re-baking (char -> library index).
"""
import struct
import sys
from pathlib import Path
from PIL import Image
import numpy as np

PALETTE = list(" .:',;,wiogOLXHWYV@")
NIGHT_PALETTE = list(" .:;,.wiogOLXHWYV@")
LIB_PALETTE = " .:;',wiogOLXHWYV@"

HERE = Path(__file__).resolve().parent
TEXDIR = HERE.parent / "globe" / "textures"


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


def write_gidx(levels_idx, lib_palette, cols, rows, out_path):
    """Write a GIDX1 binary texture.

    levels_idx: (rows, cols) array of palette indices (uint8).
    lib_palette: str of length L (1..255); level i renders as lib_palette[i].
    """
    arr = np.asarray(levels_idx, dtype=np.uint8)
    if arr.shape != (rows, cols):
        raise ValueError(f"shape {arr.shape} != ({rows}, {cols})")
    levels = len(lib_palette)
    if not 1 <= levels <= 255:
        raise ValueError(f"palette length {levels} out of range 1..255")
    if arr.min() < 0 or arr.max() >= levels:
        raise ValueError(f"index out of range for {levels} levels")
    pal_bytes = lib_palette.encode("ascii")
    if len(pal_bytes) != levels:
        raise ValueError("palette must be ASCII with no multibyte chars")
    blob = np.ascontiguousarray(arr, dtype=np.uint8).tobytes()
    assert len(blob) == rows * cols
    with open(out_path, "wb") as f:
        f.write(b"GIDX1\n")
        f.write(struct.pack("<II", cols, rows))
        f.write(struct.pack("B", levels))
        f.write(pal_bytes)
        f.write(blob)
    print(f"wrote {out_path} {cols}x{rows} levels={levels}")


def text_to_levels(txt_path, lib_palette=LIB_PALETTE):
    """Read a .txt texture and map each char to its library palette index.

    The legacy library loader reverses every text row at load time, so
    library texture space is the .txt mirrored horizontally per row:
    gidx(x, y) = lib_palette.index(txt_line[y][cols-1-x]).
    """
    lookup = {c: i for i, c in enumerate(lib_palette)}
    text = Path(txt_path).read_text()
    lines = text.split("\n")
    # committed files end with a trailing newline: drop the final empty field
    if lines and lines[-1] == "":
        lines.pop()
    rows = len(lines)
    if rows == 0:
        raise ValueError(f"{txt_path}: empty texture")
    cols = len(lines[0])
    arr = np.zeros((rows, cols), dtype=np.uint8)
    for y, line in enumerate(lines):
        if len(line) != cols:
            raise ValueError(f"{txt_path} line {y}: len {len(line)} != {cols}")
        for x, ch in enumerate(reversed(line)):
            if ch not in lookup:
                raise ValueError(f"{txt_path} line {y} col {x}: unknown char {ch!r}")
            arr[y, x] = lookup[ch]
    return arr, cols, rows


def convert_text_to_gidx(txt_path, out_path=None, lib_palette=LIB_PALETTE):
    """Convert an already-baked .txt texture to .gidx without re-baking."""
    arr, cols, rows = text_to_levels(txt_path, lib_palette)
    if out_path is None:
        p = Path(txt_path)
        out_path = str(p.with_suffix(".gidx")) if p.suffix == ".txt" else str(p) + ".gidx"
    write_gidx(arr, lib_palette, cols, rows, out_path)
    return out_path


def bake(src, palette, cols, rows, out, fn, gamma=1.0, write_text=False):
    im = Image.open(src)
    im = im.resize((cols, rows), Image.LANCZOS)
    lum = fn(im) ** gamma
    idx = dither(lum, len(palette))
    # Map bake-palette indices -> chars -> LIB_PALETTE indices so the stored
    # level renders bit-identically to the pre-refactor text path, then mirror
    # horizontally into library texture space (the loader reverses each row).
    bake_to_lib = [LIB_PALETTE.index(c) for c in palette]
    lib_idx = np.ascontiguousarray(
        np.fliplr(np.take(np.array(bake_to_lib, dtype=np.uint8), idx)), dtype=np.uint8
    )
    gidx_out = str(out)
    if gidx_out.endswith(".txt"):
        gidx_out = gidx_out[:-4] + ".gidx"
    write_gidx(lib_idx, LIB_PALETTE, cols, rows, gidx_out)
    if write_text:
        txt_out = gidx_out[:-5] + ".txt" if gidx_out.endswith(".gidx") else (str(out))
        lines = ["".join(palette[i] for i in row) for row in idx]
        open(txt_out, "w").write("\n".join(lines) + "\n")
        print(f"wrote {txt_out} {cols}x{rows}")


def main():
    args = sys.argv[1:]
    cols = int(args[args.index("--cols") + 1]) if "--cols" in args else 1440
    rows = int(args[args.index("--rows") + 1]) if "--rows" in args else 720
    write_text = "--text" in args
    texdir = TEXDIR
    texdir.mkdir(parents=True, exist_ok=True)
    if "--from-text" in args:
        i = args.index("--from-text")
        try:
            day_txt, night_txt = args[i + 1], args[i + 2]
        except IndexError:
            print("usage: bake_textures.py --from-text DAY_TXT NIGHT_TXT", file=sys.stderr)
            sys.exit(2)
        if day_txt.startswith("--") or night_txt.startswith("--"):
            print("usage: bake_textures.py --from-text DAY_TXT NIGHT_TXT", file=sys.stderr)
            sys.exit(2)
        convert_text_to_gidx(day_txt, str(Path(day_txt).with_suffix(".gidx"))
                             if Path(day_txt).suffix == ".txt" else None)
        convert_text_to_gidx(night_txt, str(Path(night_txt).with_suffix(".gidx"))
                             if Path(night_txt).suffix == ".txt" else None)
        return
    bake(HERE / "earth-day.jpg", PALETTE, cols, rows, str(texdir / "earth_hd.gidx"), luminance, gamma=0.8,
         write_text=write_text)
    # night: gamma lifts dim city glow so coasts read against black ocean
    bake(HERE / "earth-night.jpg", PALETTE, cols, rows, str(texdir / "earth_night_hd.gidx"), night_lum, gamma=0.6,
         write_text=write_text)


if __name__ == "__main__":
    main()
