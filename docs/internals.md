# globe internals

How the renderer actually works. All names and numbers below come from
`globe/src/lib.rs` and `tools/bake_textures.py`; where a number looks
arbitrary, it is quoted literally so you can grep for it.

## 1. The ray model: one ray per sub-cell sample

Each terminal cell carries a grid of sub-cell samples (1x1, 1x2 or 2x4
depending on the alphabet; see §2). Each sample casts exactly one ray.
There is no scene graph, no mesh, no depth buffer — just a ray-sphere
test, a texture lookup, and a sky function.

Per frame, `Frame::new` folds the camera, the canvas size and the
alphabet into a separable ray basis, so one sample's direction is two
fused multiply-adds per component:

```rust
// dir = vx * gx + vy * gy + v0, with (gx, gy) in sub-cell coordinates
fn dir(&self, gx: Float, gy: Float) -> [Float; 3] { ... }
```

The basis comes from the canvas geometry. With canvas size `(dw, dh)`,
cell pixels `(cp_x, cp_y)` (default `(4, 8)` in `Canvas::new`) and
sub-cell grid `(sx, sy)`:

```rust
let half_gh = (dh * sy) as Float * 0.5;
let step_y = 1.0 / half_gh;
let step_x = cp_x / cp_y * (sy as Float) / (sx as Float) * step_y;
```

Screen x runs right to left (mirrored), y top to bottom, both in units
of half the sub-cell grid height. The coefficients

```rust
let a = -step_x;
let b = (hw - 0.5) * step_x;      // hw = (dw * sx) * 0.5
let c = step_y;
let d = (0.5 - half_gh) * step_y;
```

scale the camera matrix rows into `vx = a * m[0..3]`,
`vy = c * m[4..6]`, and an offset `v0` that mixes `b`, `d` and
`-m[8..10]`. The comment in `Frame::new` explains the last term:

> The camera matrix translation is the camera position, and the ray
> origin is subtracted from the transformed direction: it cancels,
> leaving the rotation.

In other words the ray origin is the camera position `o = (x, y, z)`
itself, and the basis ends up purely rotational — no per-sample
translation survives.

The sphere test avoids normalizing the ray. `Frame::geom` returns
`(|d|^2, d.o, (d.o)^2 - |d|^2 * (|o|^2 - r^2))`: the discriminant
scaled by `|d|^2`, so every sign test is exact on the unnormalized
ray. The comment notes the saving: three multiplies where `normalize`
costs a square root and three divides. The near hit is
`t = (-d_o - sqrt(disc)) / l2`, and the surface point is `p = o + t*d`.

The ascii path (`ascii_cell`) normalizes once per cell and uses the
unit-ray form: `discriminant = (u.o)^2 - |o|^2 + r^2`,
`distance = -sqrt(discriminant) - u.o`.

The camera itself is spherical: `Camera::update(r, alpha, beta)` puts
the eye at `x = r cosA cosB, y = r sinA cosB, z = r sinB` and builds an
orthonormal frame whose rows are `(-sinA, cosA, 0)`,
`(cosA sinB, sinA sinB, -cosB)`, `(cosA cosB, sinA cosB, sinB)`, with
the position in `m[12..14]`. `CameraConfig::default` is radius 2,
angles 0 (two radii out on `+x`); `GlobeTemplate::default_zoom` is 3.5
planet radii for Saturn, 1.7 for everything else.

Day/night blending happens in `Frame::level`. With a night map enabled
the luminance is `lum = (5 * (p.SUN) / r + 0.5).clamp(0, 1)` — a 5x
slope around the subsolar point, so the terminator spans `cos = -0.1`
to `+0.1` — and the level is `(1-lum) * night + lum * day`. The sun is
treated as infinitely far away ("a million radii"), so `SUN` needs no
per-point normalize. Bodies without a night map skip this entirely:
no terminator, which is what makes the sun emissive.

## 2. Glyph alphabets: sub-cell grids, square pixels, Bayer dithering

`Glyph::sub` fixes the samples per cell: Ascii `(1, 1)`, Half `(1, 2)`,
Braille `(2, 4)`. Render resolution is therefore `sub x terminal
cells`: shrinking the font sharpens the globe at constant cost of one
ray per sample.

Square-ness comes from `step_x` above. The ratio of an x step to a y
step is `cell aspect * sy / sx`, i.e. `cp_x / cp_y * sy / sx`. With the
default 4x8 cell that ratio is 0.5 for ascii (exactly the legacy grid)
and 1.0 for half and braille — square device pixels, hence a circular
globe, in every alphabet. A non-default `char_pix` only changes the
ratio, never the code path.

Block cells are ordered-dithered, not averaged. `block_cell` casts one
ray per dot and lights the dot when the sample brightness exceeds the
Bayer 8x8 threshold at that sub-cell:

```rust
fn bayer(x: usize, y: usize) -> Float {
    (BAYER8[(y & 7) * 8 + (x & 7)] as Float + 0.5) * (1.0 / 64.0)
}
```

`BAYER8` is the standard 8x8 matrix stored row-major in `lib.rs`. The
x argument is the global sub-cell column `cx * sx + ix`, the y argument
is `(cy * sy + iy) & 7`; `bayer` masks both with `& 7` again, so the
pattern tiles every 8 sub-cells. The lit dots become a glyph via
`glyph_for`: half blocks map the 2-bit mask to `' '`, `'▀'`, `'▄'`,
`'█'`; braille maps the 8-bit mask to `U+2800 + mask`.

A blank braille cell is U+2800 (BRAILLE PATTERN BLANK), not `' '`.
`glyph_for` computes `char::from_u32(0x2800 + mask)` unconditionally,
so mask 0 yields U+2800. The dot bits come from `BRAILLE_BITS =
[[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]]`: column 0 holds
left-column dots 1,2,3,7 top to bottom, column 1 the right-column dots
4,5,6,8 — the standard 8-dot layout, which is why decoding needs no
font.

## 3. Sky: hash field, quantised bins, band envelope, sun disk, crossfade

Missed rays fall through to the sky, a pure function of the normalized
ray `u` and two quantised camera bins,
`qx = (ox * (26. + 22. * 1.5) * 8. / 10.) as Int` (and likewise `qz`).
`sky_hash` builds the field in three stages:

1. Project the ray onto the sky plane perpendicular to the sun:
   `ix = u0 - SUN0 * (u.SUN)`, `iz = u2 - SUN2 * (u.SUN)`. Parallax
   lives in world space: near depth layers pan faster.
2. Hash the ray into an integer lattice cell and extract a depth layer
   `depth = (h >> 27) & 3` (four layers), with constants 997, 571, 911
   on the way in and the mixers `2246822519`, `3266489917`,
   `2654435761`.
3. Scale the projected position per layer (`scl = 260 * (1 << depth)`),
   add the camera bins shifted by `(1 + depth) / 4` — the comment says
   the offset is rotated 90° from the sun axis so orbiting pans across
   the sky instead of into the pole — and hash again with 374761393,
   668265263, 2246822519, then `(sh ^ (sh >> 13)) * 1274126177`,
   `sh ^= sh >> 16`.

It also returns the milky-way band distance
`band_d = |u.BAND|` with `BAND = [0.399, 0.349, 0.848]`, a fixed
world-space axis independent of the camera.

The band envelope is a two-Gaussian mix in `band_env`:

```rust
g   = exp(-band_d^2 / (2 * CORE_W^2));   // CORE_W = 0.06
avg = exp(-band_d^2 / (2 * OUT_W^2));    // OUT_W  = 0.16
g * 0.8 + avg * 0.2
```

Membership cutoffs mirror the widths: `IN_BAND = 0.16 = OUT_W`,
`CORE = 0.06 = CORE_W`, plus the dust-lane half-width
`DUST_W = 0.016`. Inside the lane (`in_band && core &&
band_d < DUST_W`) dust blocks the glow: 70% of hashes go dark with
only sparse `'.'` survivors (`r = sh % 1000; r < 90`).

`sky_ascii` evaluates, in order: dust lane, sun disk, band, sparse
field. The sun sits at the fixed world direction
`SUN = [-0.7547, 0.5535, 0.3523]` (~41° off the launch view axis, so it
renders beside the earth, outside the disk): `facing = u.SUN >
0.99955` is `'*'`, the corona `> 0.99860` walks
`RAMP[((facing - 0.99860) / 0.00095 * 6.) as usize]`, and the outer
glow `> 0.99630` is `'.'`. In the band, a static per-cell tier
`pick = (sh >> 9) % 7` is scaled by the envelope,
`idx = (pick * band_env(band_d)) as usize` clamped to 6, with a rare
static core star (`core && idx == 5 && sh % 13 == 0` forces 6); the
cell shows `RAMP[idx]` with fill probability 720/1000 in the core,
200/1000 outside. The sparse field (`sh % 1000 < 30 + depth * 8`,
~3–5%) is dot-dominated via `(sh >> 24) % 13`: 0–7 `'.'`, 8–9 `':'`,
10 `';'`, 11 `'+'`, 12 `'*'` (only `depth < 2`, else `':'`).

`sky_brightness` is the same field as scalar values for the block
alphabets: dust 0.2/0.0, sun 1.0 / lerped corona / 0.3, band
`idx / 6`, field 0.7/0.8/0.9/0.95/1.0. Stars sit high on the ramp so a
single dot lights instead of smearing across a cell.

The ascii crossfade never pops. `RAMP` is
`[' ', '.', '.', ':', ';', '+', '*']` (note the doubled `'.'`);
`ramp_idx` maps `' '`→0, `'.'`→1, `':'`→3, `';'`→4, `'+'`→5,
`'*'`→6, anything else to `None` (transparent, no fade). `fade` moves
exactly one ramp step from current toward target per frame:
`RAMP[a + (b > a) - (b < a)]`. Entering the sky from a globe glyph
starts diffuse (`'*'`→`'+'`, `' '`→`' '`, else `'.'`). `Globe::render_on`
applies it only when `ascii_cell` flags the value as sky (`target =
true`); globe, fringe and ring values are written directly.

## 4. The ascii edge fringe vs block-alphabet antialiasing

Ascii has one sample per cell, so a hard hit/miss test would alias the
silhouette. Instead `ascii_cell` computes analytic sub-cell coverage
for near-miss rays. When the discriminant is negative but the ray
faces the globe (`t = -u.o > 0`), it measures how far the ray misses
by: `miss = sqrt(r^2 - discriminant) - r`, and compares against one
cell's footprint at the limb range, `foot = t * sy / (2 * half_gh)`.
If `miss < foot`, the closest-approach point `o + t*u` (normalized,
i.e. pushed onto the sphere) is sampled through `unit_texel`, and its
palette index is thinned by `coverage = 1 - miss / foot`. The limb
steps down the brightness ramp over one cell instead of clipping —
crisp at any font size, costing one sqrt plus one texture fetch on
near-miss cells only.

The block alphabets do none of this. Their antialiasing is natural:
each of the 2 or 8 sub-cell rays independently thresholds its own
brightness against the Bayer matrix (§2), so silhouettes, the
terminator, texture detail and even stars resolve at sub-cell
resolution. That is the whole point of `sub() x cells` rendering: a
smaller font is a finer ray grid.

## 5. Texture pipeline: bodies, luminance, dither, GIDX1

Each body is described by `tools/bodies/<name>.json`:

```json
{ "name": "mars", "cols": 1024, "rows": 512, "note": "...",
  "day":   { "source": "sources/mars.jpg", "url": "...",
             "luminance": "luma", "gamma": 0.9,
             "stretch": [1.0, 95.0], "window": [1, 17] },
  "night": { ... },   // earth only
  "ring":  { "source": "...", "url": "...", "inner": 1.233,
             "outer": 2.332, "tilt_deg": 0.0,
             "samples": 512, "albedo": 0.92 } }  // saturn only
```

`cols`/`rows` set the baked size (`bake_body` falls back to 1440x720
when absent); every currently checked-in spec uses 1024x512. `source`
is relative to `tools/`; missing files download from `url` with
`--fetch` into the gitignored `tools/sources/`.

`levels_for` turns a source image into dithered level indices:

- **Luminance** (`LUMINANCE`): `luma` is Rec601
  `0.299 R + 0.587 G + 0.114 B`; `earth_day` is
  `0.55 R + 0.55 G - 0.35 B` (land and ice bright, ocean dark);
  `earth_night` reuses `luma` on the city-lights map.
- **Gamma**: `lum = fn(im) ** gamma` (earth day 0.8, night 0.6, sun
  0.4, jupiter 0.8, mars/moon 0.9, the rest 1.0).
- **Percentile stretch**: `lo, hi = percentile(lum, s0, s1)`,
  `lum = (lum - lo) / (hi - lo)` clipped — e.g. jupiter/saturn
  `[1, 99]`, mars `[1, 95]`, moon `[2, 98]`, sun `[0.5, 95]`.
- **Dither target**: `palette` defaults to the 18-entry bake palette
  (earth pins its own identical copy); the sun instead confines output
  with **`window`**: `idx = round(lo + idx * (hi - lo) / (levels -
  1))`, so the sun lives at `[5, 17]` (top of the ramp) while
  mars/mercury/neptune/uranus/venus use `[1, 17]`.
- **Quantize** with `dither()`: serial Floyd–Steinberg, `buf = field *
  (levels - 1)`, round, then diffuse the error 7/16 right, 3/16
  below-left, 5/16 below, 1/16 below-right. Bake indices remap onto
  `LIB_PALETTE = " .:;',wiogOLXHWYV@"` (18 levels) via
  `bake_to_lib = [LIB_PALETTE.index(c) for c in palette]`.

Library texture space is mirrored along x: `levels_for` returns
`fliplr(idx)`, and both text paths agree — `text_to_levels` maps
`gidx(x, y) = LIB_PALETTE.index(txt_line[y][cols-1-x])`, and the
runtime loader `index_image` fills each row from `line.chars().rev()`
("columns reversed (longitude runs west)"). Ragged ascii rows pad;
unknown glyphs append to the palette (cap 255). A baked night map must
arrive with the same palette and size as the day map (`Texture::
from_baked` asserts both); ascii day+night instead share one palette
by construction. An explicit `--texture` day map replaces the whole
template (`GlobeConfig::build`), so unrelated palettes never mix.

GIDX1 is the shipping format, `include_bytes!`d into the binary — the
earth map costs no heap and no startup parse. Byte layout, little
endian:

| bytes | field |
| --- | --- |
| 0..6 | magic `"GIDX1\n"` (`BAKED_MAGIC`) |
| 6..10 | columns, u32 |
| 10..14 | rows, u32 |
| 14 | levels, u8 (1..=255) |
| 15..15+levels | palette glyphs, one ascii byte per level; level `i` renders as `palette[i]` |
| 15+levels.. | `rows * cols` level bytes, row major, first row on top, one byte per texel |

`Baked::parse` panics on a bad magic or a truncated file and then
borrows the level bytes in place; a texel fetch is a single indexed
load (`tex.day[ey * cols + ex]`).

The surface mapping is equirectangular in `Frame::texel`:
`phi = (-p2 / r * 0.5 + 0.5)` clamped, latitude from `-z`;
`theta = atan2(p1, p0) / 2π + 0.5 + angle / 2π` mod 1, longitude plus
spin; `ex = theta * tex_w`, `ey = phi * tex_h`, each clamped to
`size - 1`.

## 6. Rings: RING1, annulus intersection, shadows, tilt

Saturn's rings are a 1-D radial profile baked from the ring strip's
alpha channel (`bake_ring`: column-mean opacity, resampled by `interp`
to `samples` = 512 entries, `brightness = clip(profile * albedo)` with
`albedo` 0.92, stored as `(round(brightness*255), round(profile*255))`
pairs). RING1 layout, little endian:

| bytes | field |
| --- | --- |
| 0..6 | magic `"RING1\n"` (`RING_MAGIC`) |
| 6..10 | sample count, u32 (> 1) |
| 10..14 | inner radius, f32, in planet radii |
| 14..18 | outer radius, f32, in planet radii |
| 18..22 | tilt, f32, radians — obliquity of the ring plane about x |
| 22.. | `count` × `(brightness, opacity)` byte pairs, inner edge first, each 0..255 → 0.0..=1.0 |

Checked-in radii are inner 1.233, outer 2.332 planet radii (per the
changelog, derived from the Cassini and Encke gaps in the source
strip); `Ring::parse` requires `0 < inner < outer`. The renderer
scales them by the globe radius into `RingGeom { inner, outer,
inv_span = 1 / (outer - inner), r2 }`, with the plane normal
`axis = [0, -sin(tilt), cos(tilt)]` — the planet's spin axis.

`RingGeom::cross(o, d)` intersects the ray with the equatorial plane:
`along = d.axis`; `|along| < 1e-9` means parallel, miss. Otherwise
`t = -(o.axis) / along`; `t <= 0` is behind the camera, miss. The
crossing `p = o + t*d` lies in the plane through the origin, so its
length *is* the radius: reject `|p|^2` outside
`[inner^2, outer^2]`, else `u = (|p| - inner) * inv_span` picks sample
`min(u * (count - 1))`. Note `d` must be normalized — ring distances
compare directly against surface distances.

Occlusion falls out of comparing `t` values. `with_ring_level` blends
only when the ring crossing is nearer than the surface hit
(`t < hit`): `brightness * opacity * max_level + level * (1 -
opacity)`, i.e. compositing in palette-level space. On sky rays the
ring can only be in front, so `ring_sky` composites in ramp space
(`brightness * opacity + background * (1 - opacity)`, re-quantized via
`RAMP`), letting faint ring matter show stars through. Either way: the
ring hides the globe when nearer, the globe hides the ring when
nearer. (Saturn's `default_zoom` of 3.5 exists because from inside the
ring band the near half of the rings is behind the camera.)

Shadows run both directions:

- **Globe shaded by rings**: `Frame::level` fires `ring.cross(p,
  &SUN)` from the surface point toward the sun and scales the texel
  level by `1 - opacity` — whatever light the ring intercepts is lost.
- **Rings shaded by globe**: `RingGeom::in_planet_shadow(p)` tests
  `|p + s*SUN|^2 = r^2` for a positive root
  (`b = p.SUN`, `disc = b^2 - (|p|^2 - r^2)`, shadow iff `disc >= 0
  && (-b - sqrt(disc)) > 0`) and zeroes the ring brightness at
  shadowed crossings. All three call sites (`with_ring_level`,
  `ring_sky`, `brightness`) apply it.

Tilt moves the texture pole, not just the rings. `Frame` keeps
`tilt_cos/sin` from the ring tilt (identity when ringless) and
`planet_frame` undoes the obliquity — rotate about x by `-tilt` —
before `texel()`, so bands run parallel to the rings. Saturn is baked
with `tilt_deg = 0.0` (plane perpendicular to the spin axis, camera
orbit ~17° above it, sun ~21° above it); set 26.73 in
`tools/bodies/saturn.json` for the real obliquity.

## 7. Cost model: per-sample work, what is cached, measured numbers

One sub-cell sample costs: a `dir()` evaluation (6 multiplies, 6 adds
across the three components), a `geom()` (three dot-product-class
op sequences — no sqrt, no divide), then one of three tails. A globe
hit pays one sqrt for `t`, one `atan2` + one indexed byte load in
`texel()`, and a few multiplies for the day/night blend; a sky miss
pays the integer `sky_hash` plus, inside the band, two `exp()` calls
in `band_env`; a ringed body additionally pays a `cross()` (one sqrt)
per sample or per surface point. There is one byte of state per
sample on the way out (the braille mask accumulates in a `u8`; the
canvas itself is one `char` per cell, `y * cols + x` row-major).

Folded once per frame in `Frame::new` and reused by every sample: the
ray basis (`vx, vy, v0`), camera origin and norms (`o, oo, r, r2,
inv_r`), grid constants (`half_gh`, step-derived basis),
`max_level`, the sky bins (`qx, qz`), spin `angle`, the ring geometry
and the tilt cos/sin. Textures are borrowed in place
(`Cow::Borrowed`, no parse, no copy). What is *not* cached is the sky:
as the comment in `render_on` says, the star field hashes the live ray
direction so it pans with the camera on purpose, leaving nothing
frame-stable to cache — every sky sample is recomputed each frame,
which is also what keeps the field moving smoothly instead of
snapping.

The following figures are quoted from the performance table in
`README.md` (measured with the screensaver, output drained, at two
terminal sizes; peak RSS is `VmHWM` of the whole run — not re-measured
here), on a 4x8-cell terminal:

| grid | alphabet | cpu (one core) | peak RSS |
| --- | --- | --- | --- |
| 300x90 | ascii | 1.3% | 4.9 MB |
| 300x90 | half | 2.7% | 4.9 MB |
| 300x90 | braille | 8.0% | 5.0 MB |
| 500x150 | ascii | 3.0% | 5.3 MB |
| 500x150 | braille | 18.4% | 5.6 MB |

Rules of thumb from the same source: braille casts eight rays per cell
at about four times the cpu of the single-glyph render (roughly half
the cost per sample); the ascii path is lighter than the
single-glyph renderer the project started from; Saturn's ring
intersection adds about 14% over a ringless body at the same size; the
earth render costs around 1–3% of one core and ~5 MB resident
regardless of terminal size, since storage is per cell, not per pixel.

## Where to look

| subsystem | symbols |
| --- | --- |
| ray basis, sphere test | `Frame::new`, `Frame::dir`, `Frame::geom`, `Camera::update`, `CameraConfig` |
| alphabets, dither | `Glyph::sub`, `glyph_for`, `BRAILLE_BITS`, `BAYER8`, `bayer`, `Frame::block_cell`, `Canvas::new` (`char_pix`) |
| sky, crossfade | `SUN`, `BAND`, `CORE_W`, `OUT_W`, `DUST_W`, `IN_BAND`, `CORE`, `sky_hash`, `band_env`, `sky_ascii`, `sky_brightness`, `RAMP`, `ramp_idx`, `fade` |
| ascii fringe | `Frame::ascii_cell`, `Frame::unit_texel`, `Frame::with_ring_level`, `Frame::ring_sky` |
| surface, lighting | `Frame::level`, `Frame::texel`, `Frame::planet_frame`, `Frame::brightness` |
| textures, baking | `Baked::parse`, `BAKED_MAGIC`, `Texture::from_baked`, `index_image`, `assemble`, `GlobeTemplate::maps`, `levels_for`, `dither`, `write_gidx`, `LIB_PALETTE`, `LUMINANCE`, `earth_day`, `luma` |
| rings | `Ring::parse`, `RING_MAGIC`, `RingGeom::cross`, `RingGeom::in_planet_shadow`, `bake_ring`, `write_ring`, `tools/bodies/saturn.json` |
| frame loop, canvas | `Globe::render_on`, `Canvas`, `GlobeConfig::build`, `GlobeTemplate::default_zoom` |
