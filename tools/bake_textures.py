#!/usr/bin/env python3
"""Build globe textures (GIDX1) for the sun, planets and the moon.

Every body is described by `tools/bodies/<name>.json`:

    {
      "name": "mars",
      "cols": 1440, "rows": 720,
      "note": "free text",
      "day":   {"source": "sources/mars.jpg", "url": "https://.../2k_mars.jpg",
                "luminance": "luma", "gamma": 1.0,
                "stretch": [1.0, 99.0], "window": [1, 17]},
      "night": null
    }

`source` is a path relative to `tools/`; missing files are downloaded from
`url` when `--fetch` is given. `luminance` is `luma` (Rec601), `earth_day`
(land and ice bright, ocean dark) or `earth_night` (city lights). `gamma`
shapes the response, `stretch` clip percentiles of the luminance to 0..1 so
low contrast maps still use the whole ramp, and `window` confines the output
to a level range of the palette (the sun lives at the top of the ramp).
Maps are quantized with Floyd-Steinberg dithering, then mirrored into
library texture space (the ascii loader reverses rows).

Planet imagery: Solar System Scope, https://www.solarsystemscope.com/textures/
(CC BY 4.0).

Usage:
  python3 bake_textures.py --list
  python3 bake_textures.py --body mars [--fetch] [--text] [--cols N] [--rows N]
  python3 bake_textures.py --all [--fetch]
  python3 bake_textures.py --from-text DAY_TXT NIGHT_TXT
"""
import json
import struct
import sys
import urllib.request
from pathlib import Path

from PIL import Image
import numpy as np

# Palette glyphs as the library stores them: level i renders as LIB_PALETTE[i].
LIB_PALETTE = " .:;',wiogOLXHWYV@"
# Bake palettes: the dither target. Distinct from LIB_PALETTE only in that the
# original earth maps carried duplicate glyphs; every glyph maps back to the
# library palette, so the rendered character is preserved either way.
PALETTE = list(" .:',;,wiogOLXHWYV@")

HERE = Path(__file__).resolve().parent
TEXDIR = HERE.parent / "globe" / "textures"
BODYDIR = HERE / "bodies"
SRCDIR = HERE / "sources"


def luma(im):
    """Rec601 luminance: neutral for anything that is not earth."""
    a = np.asarray(im.convert("RGB"), dtype=np.float32) / 255.0
    return (0.299 * a[..., 0] + 0.587 * a[..., 1] + 0.114 * a[..., 2]).clip(0, 1)


def earth_day(im):
    """Land bright, ocean dark: green/red weighted, blue suppressed."""
    a = np.asarray(im.convert("RGB"), dtype=np.float32) / 255.0
    return (0.55 * a[..., 0] + 0.55 * a[..., 1] - 0.35 * a[..., 2]).clip(0, 1)


def earth_night(im):
    """City lights: the same neutral luminance, the dither does the rest."""
    return luma(im)


LUMINANCE = {"luma": luma, "earth_day": earth_day, "earth_night": earth_night}


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
    if arr.max() >= levels:
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
    lines = Path(txt_path).read_text().split("\n")
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


def body_path(name):
    return BODYDIR / f"{name}.json"


def load_body(name):
    path = body_path(name)
    if not path.exists():
        raise SystemExit(f"unknown body {name!r}: {path} missing (see --list)")
    spec = json.loads(path.read_text())
    spec["name"] = name
    return spec


def list_bodies():
    return sorted(p.stem for p in BODYDIR.glob("*.json"))


def ensure_source(map_spec, fetch):
    """Returns the path to the map's source image, downloading if allowed."""
    src = HERE / map_spec["source"]
    if src.exists():
        return src
    url = map_spec.get("url")
    if not url:
        raise SystemExit(f"missing {src} and no url in the body spec")
    if not fetch:
        raise SystemExit(f"missing {src}: rerun with --fetch to download {url}")
    src.parent.mkdir(parents=True, exist_ok=True)
    print(f"fetching {url} -> {src}")
    with urllib.request.urlopen(url, timeout=60) as rsp, open(src, "wb") as out:
        out.write(rsp.read())
    return src


def levels_for(map_spec, cols, rows, fetch):
    """Source image -> dithered library-level map, in library texture space."""
    src = ensure_source(map_spec, fetch)
    im = Image.open(src).convert("RGB").resize((cols, rows), Image.LANCZOS)
    fn = LUMINANCE[map_spec.get("luminance", "luma")]
    lum = fn(im) ** float(map_spec.get("gamma", 1.0))

    stretch = map_spec.get("stretch")
    if stretch:
        lo, hi = np.percentile(lum, float(stretch[0])), np.percentile(lum, float(stretch[1]))
        if hi - lo < 1e-6:
            raise SystemExit(f"{src}: flat luminance (p{stretch[0]}={lo:.4f}, p{stretch[1]}={hi:.4f})")
        lum = ((lum - lo) / (hi - lo)).clip(0, 1)

    palette = list(map_spec.get("palette", PALETTE))
    bake_to_lib = np.array([LIB_PALETTE.index(c) for c in palette], dtype=np.uint8)
    idx = bake_to_lib[dither(lum, len(palette))]

    window = map_spec.get("window")
    if window:
        levels = len(palette)
        lo, hi = int(window[0]), int(window[1])
        if not 0 <= lo < hi <= len(LIB_PALETTE) - 1:
            raise SystemExit(f"{src}: window {window} outside 0..{len(LIB_PALETTE) - 1}")
        idx = np.rint(lo + idx.astype(np.float32) * (hi - lo) / (levels - 1)).astype(np.uint8)

    return np.ascontiguousarray(np.fliplr(idx), dtype=np.uint8)


def bake_map(spec, key, name, cols, rows, fetch, write_text):
    map_spec = spec.get(key)
    if not map_spec:
        return None
    idx = levels_for(map_spec, cols, rows, fetch)
    suffix = "_night" if key == "night" else ""
    out = TEXDIR / f"{name}{suffix}_hd.gidx"
    write_gidx(idx, LIB_PALETTE, cols, rows, str(out))
    if write_text:
        txt = out.with_suffix(".txt")
        rows_text = ["".join(LIB_PALETTE[i] for i in row) for row in idx]
        txt.write_text("\n".join(rows_text) + "\n")
        print(f"wrote {txt} {cols}x{rows}")
    return out


def bake_body(name, fetch, write_text, cols=None, rows=None):
    spec = load_body(name)
    cols = cols or int(spec.get("cols", 1440))
    rows = rows or int(spec.get("rows", 720))
    TEXDIR.mkdir(parents=True, exist_ok=True)
    for key in ("day", "night"):
        bake_map(spec, key, name, cols, rows, fetch, write_text)


def main():
    args = sys.argv[1:]
    if "--list" in args:
        for name in list_bodies():
            spec = json.loads(body_path(name).read_text())
            night = " +night" if spec.get("night") else ""
            print(f"{name:10s} {spec.get('note', '')}{night}")
        return
    if "--from-text" in args:
        i = args.index("--from-text")
        if len(args) < i + 3:
            raise SystemExit("usage: bake_textures.py --from-text DAY_TXT NIGHT_TXT")
        day_txt, night_txt = args[i + 1], args[i + 2]
        for txt in (day_txt, night_txt):
            out = Path(txt).with_suffix(".gidx") if Path(txt).suffix == ".txt" else None
            convert_text_to_gidx(txt, str(out) if out else None)
        return

    fetch = "--fetch" in args
    write_text = "--text" in args
    cols = int(args[args.index("--cols") + 1]) if "--cols" in args else None
    rows = int(args[args.index("--rows") + 1]) if "--rows" in args else None
    if "--all" in args:
        for name in list_bodies():
            bake_body(name, fetch, write_text, cols, rows)
        return
    if "--body" not in args:
        raise SystemExit(
            "usage: bake_textures.py (--body NAME | --all | --list) [--fetch] [--text]"
        )
    for name in args[args.index("--body") + 1:]:
        if name.startswith("--"):
            break
        bake_body(name, fetch, write_text, cols, rows)


if __name__ == "__main__":
    main()
