# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No changes yet.

## [0.3.0] - 2026-09-20

The fork's rendering and bodies overhaul, on top of upstream v0.2.1.

### Added

- Sub-cell glyph alphabets via `-G` / `--glyph`: `braille` (default, 2x4
  samples per cell), `half` (1x2 upper/lower blocks), `ascii` (one palette
  glyph per cell, the original look). Sub-cell samples are ordered-dithered
  with a Bayer 8x8 matrix; the ray grid keeps square device pixels so the
  globe stays circular in every alphabet.
- Ten switchable bodies via `-t` / `--template`: `earth` (default), `sun`,
  `mercury`, `venus`, `moon`, `mars`, `jupiter`, `saturn`, `uranus`,
  `neptune`. Earth keeps its day + night map pair; every other body carries
  a single 1440x720 palette-index map.
- Saturn ring system: a `RING1` radial `(brightness, opacity)` profile baked
  from the ring strip's alpha channel (inner radius 1.233, outer 2.332 planet
  radii), with radii derived from the Cassini and Encke gaps in the source
  strip. The renderer ray traces the annulus in the planet's equatorial plane
  with exact ray intersection: the ring hides the globe, the globe hides the
  ring, the planet casts a shadow across the rings and the rings cast a
  shadow across the bands. The profile carries the ring plane tilt (Saturn is
  baked with the plane perpendicular to the spin axis, which puts the camera's
  equatorial orbit about 17° above it and the sun about 21° above it); set
  `tilt_deg` to 26.73 in `tools/bodies/saturn.json` to model the real
  obliquity instead.
- `--texture` / `--texture-night` now actually load custom ascii maps on top
  of the template, as the README always advertised: a custom day map replaces
  the template, a missing file exits 1 with a message instead of panicking,
  and a night map of the wrong size fails fast with an explicit message.
- Compact baked `GIDX1` textures (`magic, cols u32le, rows u32le, levels u8,
  palette bytes, rows*cols level bytes`), `include_bytes!`d straight into the
  binary: no parse, no copy, no per-texel heap. Maps are baked at 1024x512
  (11 maps, 5.6 MB, 7 MB binary) rather than 1440x720: half the repository and
  binary weight for a dot grid that stays ~1:1 with the texture up to a
  ~180-row terminal, and `--cols`/`--rows` re-bake higher on demand.
- Lightweight demo assets: five 1-bit GIFs recorded from real pty captures
  (screensaver, body tour, the three alphabets, interactive mode, saturn's
  rings), 454 KB together, replacing the 3.2 MB upstream drag recording; the
  repository ignores the baker's optional text dumps, `__pycache__`, `*.pyc`,
  editor and OS noise, and the unreferenced legacy `earth_night.txt` map is
  gone.
- Texture maps baked at 1024x512 instead of 1440x720: 5.6 MB of maps and a
  7 MB binary instead of 11.1 MB and 12.7 MB.
- Body-driven texture baker: each body is described by
  `tools/bodies/<name>.json` (`source`, `url`, `luminance`, `gamma`,
  `stretch`, `window`, optional `night` and `ring` sections; schema in the
  baker's docstring). `python3 tools/bake_textures.py --list` shows the ten
  bodies; `--fetch` downloads missing sources into the gitignored
  `tools/sources/`.
- Analytic limb fringe: near-miss rays sample the limb surface point and thin
  the palette index by sub-cell coverage, so the silhouette steps down the
  brightness ramp instead of hard clipping.
- Planet, moon and sun imagery from
  [Solar System Scope](https://www.solarsystemscope.com/textures/), used
  under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

### Changed

- Canvas and diff buffers hold one glyph per terminal cell (flat `Vec<char>`);
  the fullscreen diff printer batches changed cells into runs with one write
  per frame and skips the flush when nothing changed. Default refresh rate is
  now 30 fps.
- Faster ray/sampling math: per-frame separable ray basis instead of a
  per-sample normalized transform, sphere test via scaled discriminant (no
  normalize, no divide), lighting without a square root.
- The sun is emissive (no night map, no terminator) and `-n` is a no-op for
  every body except earth.
- `-z` / `--cam-zoom` defaults to the body's recommended camera distance via
  `GlobeTemplate::default_zoom()`: 3.5 planet radii for Saturn (a ringed
  planet needs room — from inside the ring band the near half of the rings
  is behind the camera), 1.7 for everything else.

### Fixed

- The windowed modes (`-i`, `-p`) no longer walk one row down the screen per
  frame: the cursor returns to the canvas origin, so the globe redraws in
  place instead of smearing across the terminal (visible in the old upstream
  drag recording).
- Terminator orientation: the light vector ran from the sun to the surface,
  against its own comment, so the day side was inverted; the day map now
  lands on the sun-facing hemisphere, matching the sun drawn in the sky.
- Odd-sized grids are centred on the true grid centre instead of the old
  integer half-grid bias.

## [0.2.1]

- Upgraded `clap` dependency to `3.0.0`.
- Changed `globe-cli` `template` argument to not be required.

## [0.2.0]

- Added multiple CLI arguments for setting up the scene (`refresh-rate`,
  `globe-rotation`, `cam-rotation`, `cam-zoom`, `location`, `focus-speed`,
  `night`, `template`, `texture`, `texture-night`).
- Added experimental *listing mode* that supports reading coordinates from
  standard input and going through all of them, animating camera target
  changes (see `--pipe`).
- Enabled ability to display night side of the globe using an additional
  texture.
- Changed default Earth texture (now includes New Zealand).
- Added vim-style navigation for the interactive mode.
- Improved internal library representation of `Texture`.
- Improved documentation.

## [0.1.2]

- Added clearing screen on exit.
- Fixed panic when using rust version < 1.45.

## [0.1.1]

- Fixed mouse capture staying on after exit.

## [0.1.0]

- Initial release.

[Unreleased]: https://github.com/adamsky/globe/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/adamsky/globe/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/adamsky/globe/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/adamsky/globe/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/adamsky/globe/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/adamsky/globe/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/adamsky/globe/releases/tag/v0.1.0
