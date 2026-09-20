# Contributing to globe

Thanks for your interest in contributing. This project is GPL-3.0; **by
submitting a pull request you agree to license your contribution under
GPL-3.0** (see `LICENSE`). There is no Developer Certificate of Origin and
nothing to sign — the PR itself is the agreement.

## Build and run

You need the [Rust toolchain](https://rustup.rs) (the workspace currently
builds on rustc 1.93.1) and Python 3 with PIL + numpy for the texture baker.

```sh
cargo build --workspace
cargo run -q -p globe-cli -- --help
cargo run -q -p globe-cli -- -s -t mars
cargo run -q -p globe-cli -- -s -G ascii
```

All three commands above were verified against the working tree
(`--help` prints the full option listing, `--version` reports 0.3.0).
Screensaver (`-s`), interactive (`-i`) and listing (`-p`) modes need a real
terminal and were **not** exercised headlessly here.

## Checks CI enforces

CI (`.github/workflows/ci.yml`) runs these four steps on Ubuntu, macOS and
Windows; a PR must pass all of them:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
cargo test --workspace
```

Notes from the current tree (2026-09-20):

- `cargo test --workspace` passes (three targets, 0 tests each — there is no
  test suite yet, only the harness compiling).
- `cargo fmt --all --check` currently **fails** on the in-progress rings work
  in `globe/src/lib.rs` (Bayer table rewrapping) and the new
  `globe/examples/ringcheck.rs`. Do not "fix" this by reformatting files
  someone else is editing — coordinate first, and never run a project-wide
  `cargo fmt` (without `--check`) while others have uncommitted changes.
- `cargo clippy --workspace --all-targets -- -D warnings` currently
  **fails** with 6 deny-by-default lints in the new renderer code
  (`map_or` simplifications, indexed loop variables, `should_implement_trait`
  on a `default` method). Plain `cargo clippy` (warn mode) passes.

Do not run project-wide formatters or `cargo fix` on files you do not own.

## Adding a body end to end

Textures are baked into `globe/textures/*.gidx` (`GIDX1` format: magic, cols
u32le, rows u32le, levels u8, palette bytes, then `rows*cols` level bytes)
and `include_bytes!`d, so they ship inside the packaged crate — a new `.gidx`
must be committed, and nothing outside `globe/textures/` needs packaging
help. `tools/` holds the Python baker (needs PIL+numpy, both verified
present) and `tools/bodies/*.json` specs; `tools/sources/` is gitignored and
fetched on demand from Solar System Scope (CC BY 4.0).

1. Drop a 2:1 equirectangular image into `tools/sources/`, e.g.
   `tools/sources/europa.jpg`.
2. Describe it in `tools/bodies/europa.json`. The schema is the baker's
   docstring (`tools/bake_textures.py`, top of file); per-body fields are:

   ```json
   {
     "name": "europa",
     "cols": 1440, "rows": 720,
     "note": "free text shown by --list",
     "day":   {"source": "sources/europa.jpg",
               "url": "https://.../2k_europa.jpg",
               "luminance": "luma", "gamma": 1.0,
               "stretch": [1.0, 99.0], "window": [1, 17]},
     "night": null
   }
   ```

   - `source` is a path relative to `tools/`; a missing file is downloaded
     from `url` only when `--fetch` is given, otherwise the baker exits 1.
   - `luminance` is `luma` (Rec601, neutral), `earth_day` (land/ice bright,
     ocean dark) or `earth_night` (city lights).
   - `gamma` shapes the response; `stretch` clips luminance percentiles to
     0..1 so low-contrast maps still use the whole ramp; `window` confines
     output to a level range of the palette (the sun lives at the top).
   - `palette` optionally overrides the bake palette (earth does).
   - A `"ring"` section (`source`, `url`, `inner`/`outer` radii in planet
     radii, `tilt_deg`, `samples`, `albedo`) bakes a `RING1` radial
     `(brightness, opacity)` profile from the strip image's alpha channel —
     see `tools/bodies/saturn.json`.

3. Bake it (verified: `--list` lists all ten bodies; a sandboxed
   `--body moon` rebake byte-compares the workflow; `--fetch` was **not**
   run — it needs network — and neither was `--all`, which rewrites every
   committed texture):

   ```sh
   python3 tools/bake_textures.py --list
   python3 tools/bake_textures.py --body europa
   ```

   Add `--fetch` if the source must be downloaded, `--text` for a debug
   `.txt` dump, `--cols N --rows N` to override the spec size. Unknown names
   fail fast: `--body nosuchbody` exits 1 with
   `unknown body 'nosuchbody': ... missing (see --list)` (verified).

4. Wire the baked map into the library (`globe/src/lib.rs`): add the
   `GlobeTemplate` variant, its entry in `GlobeTemplate::ALL`, its lowercase
   `name()`, its arm in `maps()` (plus a `SATURN_RING`-style static if it has
   a ring profile), and the `include_bytes!("../textures/europa_hd.gidx")`
   static next to the other ten.
5. No CLI change is needed: `-t/--template` validates against
   `GlobeTemplate::NAMES`, so the new body is accepted automatically.
   Verify with `cargo run -q -p globe-cli -- -t europa` (invalid names are
   rejected by clap — verified with `-t pluto`) and `cargo test --workspace`.

## Adding a glyph alphabet

Glyph alphabets live in `globe/src/lib.rs`; the CLI flag is `-G/--glyph` in
`globe-cli/src/main.rs`. The current three are `ascii` (1 sample/cell, the
original direct-palette path via `ascii_cell`), `half` (1x2 blocks) and
`braille` (2x4 dots, the default). A new alphabet needs all of these:

1. A `Glyph` enum variant plus its `sub()` grid size, `name()` and
   `from_name()` arms.
2. A bit-layout table like `BRAILLE_BITS` and a rendering arm in `glyph_for`;
   block alphabets render through `block_cell` (Bayer-ordered dither of one
   ray per sub-cell into one glyph), while `ascii` keeps its single-centre-
   sample path in `render_on` — decide which path the new alphabet belongs
   to.
3. The new name in the CLI's `.possible_values([...])` for `--glyph` (the
   `Glyph::from_name(...).expect("unknown glyph mode")` wiring picks it up
   from there). Verify invalid names are still rejected (`-G bogus` exits
   with clap's possible-values error — verified).

## Commit messages

There is no enforced hook; follow the existing `git log --oneline` style: a
short (under ~72 chars) summary line, ideally with the area prefix used in
this tree (`templates:`, `sky:`, `screensaver:`, `docs:`, ...), then a blank
line and a body explaining *why* and how the change was verified (frames
compared, texels round-tripped, commands run). One logical change per
commit; do not mix a feature with a reformat.

## Reporting bugs

Use the issue forms (bug report / feature request). Include the glyph mode
(`-G`), body (`-t`), terminal size, and the exact command line — the forms
ask for all of it.
