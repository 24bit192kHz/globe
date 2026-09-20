//! Customizable ASCII globe generator.
//!
//! Based on [C++ code from DinoZ1729](https://github.com/DinoZ1729/Earth).
//!
//! # Rendering model
//!
//! A [`Canvas`] holds one glyph per terminal cell. [`Glyph`] picks how many
//! surface samples each of those cells carries: `Ascii` samples once per
//! cell, `Half` twice (upper/lower half blocks), `Braille` eight times
//! (2x4 dots). Sub-cell samples are ordered-dithered into block glyphs, so
//! render resolution is `sub() x terminal cells`: the smaller the terminal
//! font gets, the finer the globe, while the cost stays one ray per sample
//! and one byte of state per sample.

use std::borrow::Cow;
use std::f32::consts::PI;
use std::fs::File;
use std::io::Read;

pub type Int = isize;
pub type Float = f32;

/// Sky brightness ramp, dim -> bright. Crossfade walks this one step
/// per frame toward target: ultra-smooth, never pops or flashes.
const RAMP: [char; 7] = [' ', '.', '.', ':', ';', '+', '*'];

/// Sun direction, unit length. Sits ~41 deg off the launch view axis:
/// visible beside earth, outside the globe disk. Light matches sun dir so
/// the lit side is coherent.
const SUN: [Float; 3] = [-0.7547, 0.5535, 0.3523];

/// Fixed world-space milky way band axis, unit length (camera independent).
const BAND: [Float; 3] = [0.399, 0.349, 0.848];

/// Milky way envelope: bright core half-width (~7 deg full).
const CORE_W: Float = 0.06;
/// Milky way envelope: faint band edge (~18 deg full).
const OUT_W: Float = 0.16;
/// Great rift (dark lane) half-width.
const DUST_W: Float = 0.016;
/// Band membership cutoff, matches [`OUT_W`].
const IN_BAND: Float = 0.16;
/// Core membership cutoff, matches [`CORE_W`].
const CORE: Float = 0.06;

/// Bayer 8x8 ordered dither thresholds, row major. Sub-cell sample
/// `(x, y)` lights its dot when its brightness exceeds
/// `(BAYER8[(y % 8) * 8 + x % 8] + 0.5) / 64`.
const BAYER8: [u8; 64] = [
    0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38,
    60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25,
    15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
];

/// Braille dot bit for sub-cell `(column, row)` in the 8-dot layout.
const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Sub-cell glyph alphabet: how many samples one character cell carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    /// One sample per cell, palette glyphs. The original look, and the
    /// reference the block alphabets are measured against.
    Ascii,
    /// 1x2 samples per cell: ` `, `▀`, `▄`, `█`.
    Half,
    /// 2x4 samples per cell: 8-dot braille, the finest of the three.
    Braille,
}

impl Glyph {
    /// Samples per cell as `(columns, rows)`.
    pub fn sub(self) -> (usize, usize) {
        match self {
            Glyph::Ascii => (1, 1),
            Glyph::Half => (1, 2),
            Glyph::Braille => (2, 4),
        }
    }

    /// Stable lowercase name, as accepted by [`Glyph::from_name`].
    pub fn name(self) -> &'static str {
        match self {
            Glyph::Ascii => "ascii",
            Glyph::Half => "half",
            Glyph::Braille => "braille",
        }
    }

    /// Parses a [`Glyph::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ascii" => Some(Glyph::Ascii),
            "half" => Some(Glyph::Half),
            "braille" => Some(Glyph::Braille),
            _ => None,
        }
    }
}

/// Glyph for a sub-cell dot mask (space when nothing is lit).
#[inline]
fn glyph_for(glyph: Glyph, mask: u8) -> char {
    match glyph {
        Glyph::Braille => char::from_u32(0x2800 + mask as u32).unwrap_or(' '),
        Glyph::Half => match mask & 0b11 {
            0b01 => '▀',
            0b10 => '▄',
            0b11 => '█',
            _ => ' ',
        },
        Glyph::Ascii => ' ',
    }
}

/// Ramp index of a char. Unknown chars act as transparent (no fade).
fn ramp_idx(c: char) -> Option<usize> {
    match c {
        ' ' => Some(0),
        '.' => Some(1),
        ':' => Some(3),
        ';' => Some(4),
        '+' => Some(5),
        '*' => Some(6),
        _ => None,
    }
}

/// Crossfade walk: one ramp step from `cur` toward `target`. Chars appear
/// and disappear as `' ' -> '.' -> ':' -> ';' -> '+' -> '*'`, which reads
/// as diffuse glow instead of a hard pop.
#[inline]
fn fade(cur: char, target: char) -> char {
    if cur == target {
        return cur;
    }
    match (ramp_idx(cur), ramp_idx(target)) {
        (Some(a), Some(b)) => RAMP[a + (b > a) as usize - (b < a) as usize],
        // entering sky from a globe char: start diffuse
        _ => {
            if target == ' ' {
                ' '
            } else if target == '*' {
                '+'
            } else {
                '.'
            }
        }
    }
}

/// Magic of a baked texture image.
const BAKED_MAGIC: &[u8; 6] = b"GIDX1\n";

/// Baked texture image: compact palette-index data, used in place.
///
/// The file is self describing, little endian:
///
/// ```text
/// 0..6             magic "GIDX1\n"
/// 6..10            columns, u32
/// 10..14           rows, u32
/// 14               levels, u8
/// 15..15 + levels  palette glyphs, one ascii byte per level
/// 15 + levels..    rows * cols level bytes, row major, first row on top
/// ```
///
/// Texels are stored in *library texture space*, which is the readable
/// ascii image mirrored along x (the ascii loader reverses rows). One byte
/// holds one texel, so [`GlobeTemplate::Earth`] borrows the data straight
/// out of the executable: no parse, no copy, no per-texel heap.
#[derive(Clone)]
pub struct Baked {
    data: &'static [u8],
    palette: Vec<char>,
    size: (usize, usize),
}

impl Baked {
    /// Parses a baked image, panicking on a malformed file.
    pub fn parse(data: &'static [u8]) -> Self {
        assert!(
            data.len() >= 15 && &data[..6] == BAKED_MAGIC,
            "not a GIDX1 texture"
        );
        let cols = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
        let rows = u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;
        let levels = data[14] as usize;
        let end = 15 + levels + rows * cols;
        assert!(
            levels > 0 && cols > 0 && rows > 0 && data.len() >= end,
            "truncated GIDX1 texture"
        );
        Self {
            data: &data[15 + levels..end],
            palette: data[15..15 + levels].iter().map(|&b| b as char).collect(),
            size: (cols, rows),
        }
    }

    /// Texture dimensions in texels.
    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    /// Palette glyphs: level `i` renders as `palette()[i]`.
    pub fn palette(&self) -> &[char] {
        &self.palette
    }
}

/// Globe texture: one palette level per texel.
///
/// Levels are bytes over one flat allocation, four times smaller than the
/// old vector of char rows, and a texel fetch is a single indexed load.
/// Baked textures borrow straight out of the executable, so the earth map
/// costs no heap and no startup parse at all.
pub struct Texture {
    day: Cow<'static, [u8]>,
    night: Option<Cow<'static, [u8]>>,
    palette: Vec<char>,
    size: (usize, usize),
}

impl Texture {
    /// Builds a texture from a baked day image and an optional baked night
    /// image. Both must share palette and size so levels can be blended.
    pub fn from_baked(day: &Baked, night: Option<&Baked>) -> Self {
        if let Some(night) = night {
            assert_eq!(
                night.palette, day.palette,
                "day and night textures need the same palette"
            );
            assert_eq!(
                night.size, day.size,
                "day and night textures need the same size"
            );
        }
        Self {
            day: Cow::Borrowed(day.data),
            night: night.map(|n| Cow::Borrowed(n.data)),
            palette: day.palette.clone(),
            size: day.size,
        }
    }

    /// Builds a texture from an ascii image: one glyph per texel, first row
    /// on top, columns reversed (longitude runs west). Ragged rows are
    /// padded and glyphs missing from the palette are appended to it, so
    /// any image renders exactly as written.
    pub fn from_ascii(image: &str, palette: Option<Vec<char>>) -> Self {
        let mut palette = palette.unwrap_or_default();
        let (day, size) = index_image(image, &mut palette);
        Self {
            day: Cow::Owned(day),
            night: None,
            palette,
            size,
        }
    }

    /// Sets the night side image, indexed with this texture's palette.
    pub fn set_night_ascii(&mut self, image: &str) {
        let (night, size) = index_image(image, &mut self.palette);
        assert_eq!(
            size, self.size,
            "night texture must match the day texture size"
        );
        self.night = Some(Cow::Owned(night));
    }

    /// Sets the night side from a baked image.
    pub fn set_baked_night(&mut self, night: Baked) {
        assert_eq!(
            night.palette, self.palette,
            "day and night textures need the same palette"
        );
        assert_eq!(
            night.size, self.size,
            "day and night textures need the same size"
        );
        self.night = Some(Cow::Borrowed(night.data));
    }

    /// Texture dimensions in texels.
    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    /// Palette glyphs.
    pub fn palette(&self) -> &[char] {
        &self.palette
    }
}

/// Indexes an ascii image into palette levels, appending unknown glyphs.
fn index_image(image: &str, palette: &mut Vec<char>) -> (Vec<u8>, (usize, usize)) {
    let lines: Vec<&str> = image.lines().collect();
    let rows = lines.len();
    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut out = vec![0u8; cols * rows];
    for (y, line) in lines.iter().enumerate() {
        for (x, c) in line.chars().rev().enumerate() {
            out[y * cols + x] = match find_index(c, palette) {
                i if i >= 0 => i as u8,
                _ => {
                    palette.push(c);
                    assert!(palette.len() <= 255, "palette overflow");
                    (palette.len() - 1) as u8
                }
            };
        }
    }
    (out, (cols, rows))
}

/// Canvas that will be used to render the globe onto.
///
/// One glyph per terminal cell, row major: cell `(x, y)` lives at
/// `y * cols + x`. Storage is exactly the cell grid, so a full screen costs
/// one byte per cell instead of one glyph per device pixel.
pub struct Canvas {
    /// Cell glyphs, row major.
    pub matrix: Vec<char>,
    size: (usize, usize),
    /// Approximate terminal cell size in device pixels. Only the aspect
    /// ratio matters: it keeps the globe circular on non-square cells.
    pub char_pix: (usize, usize),
}

impl Canvas {
    /// Creates a canvas `cols` by `rows` characters.
    pub fn new(cols: u16, rows: u16, cell_px: Option<(usize, usize)>) -> Self {
        let (cols, rows) = (cols as usize, rows as usize);
        Self {
            matrix: vec![' '; cols * rows],
            size: (cols, rows),
            char_pix: cell_px.unwrap_or((4, 8)),
        }
    }

    /// Canvas size in characters.
    pub fn get_size(&self) -> (usize, usize) {
        self.size
    }

    /// One row of glyphs.
    pub fn row(&self, y: usize) -> &[char] {
        let w = self.size.0;
        &self.matrix[y * w..y * w + w]
    }

    /// Blanks every cell.
    pub fn clear(&mut self) {
        self.matrix.fill(' ');
    }
}

/// Main globe abstraction.
pub struct Globe {
    pub camera: Camera,
    pub radius: Float,
    pub angle: Float,
    pub texture: Texture,
    pub display_night: bool,
    pub frame: u64,
    /// Sub-cell glyph alphabet, see [`Glyph`].
    pub glyph: Glyph,
}

impl Globe {
    /// Renders the globe onto the given canvas, one glyph per cell.
    pub fn render_on(&mut self, canvas: &mut Canvas) {
        self.frame = self.frame.wrapping_add(1);
        let (dw, dh) = canvas.get_size();
        let (tex_w, tex_h) = self.texture.size;
        if dw == 0 || dh == 0 || tex_w == 0 || tex_h == 0 {
            return;
        }
        let (sx, sy) = self.glyph.sub();
        let ascii = self.glyph == Glyph::Ascii;
        // The star field pans with the camera on purpose (it hashes the
        // live ray direction), so there is nothing frame-stable to cache:
        // every sky sample is recomputed, which is also what keeps the
        // field moving smoothly instead of snapping.
        let f = Frame::new(self, canvas, sx, sy);

        for cy in 0..dh {
            let gy0 = (cy * sy) as Float;
            let row = &mut canvas.matrix[cy * dw..cy * dw + dw];
            for cx in 0..dw {
                let gx0 = (cx * sx) as Float;
                let (ch, target) = if ascii {
                    f.ascii_cell(gx0, gy0)
                } else {
                    (f.block_cell(cx, cy, gx0, gy0), false)
                };
                row[cx] = if target { fade(row[cx], ch) } else { ch };
            }
        }
    }
}

/// Per-frame sampler: ray grid, camera and texture constants are folded
/// once per frame, so one sub-cell sample is a handful of multiplies.
struct Frame<'a> {
    tex: &'a Texture,
    night: bool,
    glyph: Glyph,
    /// Sub-cell grid per cell.
    sx: usize,
    sy: usize,
    /// Ray direction basis: `dir = vx * gx + vy * gy + v0`.
    vx: [Float; 3],
    vy: [Float; 3],
    v0: [Float; 3],
    /// Camera origin and norms.
    o: [Float; 3],
    oo: Float,
    r: Float,
    r2: Float,
    inv_r: Float,
    /// Half the sub-cell grid height, in sub-cells.
    half_gh: Float,
    /// Highest palette level, as float.
    max_level: Float,
    /// Sky quantization bins.
    qx: Int,
    qz: Int,
    /// Globe spin.
    angle: Float,
}

impl<'a> Frame<'a> {
    fn new(g: &'a Globe, canvas: &Canvas, sx: usize, sy: usize) -> Self {
        let (dw, dh) = canvas.get_size();
        let (cp_x, cp_y) = (canvas.char_pix.0 as Float, canvas.char_pix.1 as Float);
        let half_gh = (dh * sy) as Float * 0.5;
        let step_y = 1.0 / half_gh;
        // Sub-cell samples cover square device pixels, so one sub-cell step
        // in x is `cell aspect * sy / sx` of a step in y. With the default
        // 4x8 cell that is 0.5 for ascii (exactly the legacy grid) and 1.0
        // for half blocks and braille: a circular globe in every alphabet.
        let step_x = cp_x / cp_y * (sy as Float) / (sx as Float) * step_y;
        let hw = (dw * sx) as Float * 0.5;

        let m = g.camera.matrix;
        let (ox, oy, oz) = (g.camera.x, g.camera.y, g.camera.z);
        // screen x runs right to left (mirrored), y top to bottom, both in
        // units of half the sub-cell grid height.
        let a = -step_x;
        let b = (hw - 0.5) * step_x;
        let c = step_y;
        let d = (0.5 - half_gh) * step_y;
        // The camera matrix translation is the camera position, and the ray
        // origin is subtracted from the transformed direction: it cancels,
        // leaving the rotation.
        let vx = [a * m[0], a * m[1], a * m[2]];
        let vy = [c * m[4], c * m[5], c * m[6]];
        let v0 = [
            b * m[0] + d * m[4] - m[8],
            b * m[1] + d * m[5] - m[9],
            b * m[2] + d * m[6] - m[10],
        ];

        let r = g.radius;
        Self {
            tex: &g.texture,
            night: g.display_night,
            glyph: g.glyph,
            sx,
            sy,
            vx,
            vy,
            v0,
            o: [ox, oy, oz],
            oo: ox * ox + oy * oy + oz * oz,
            r,
            r2: r * r,
            inv_r: 1.0 / r,
            half_gh,
            max_level: (g.texture.palette.len() - 1) as Float,
            qx: (ox * (26. + 22. * 1.5) * 8. / 10.) as Int,
            qz: (oz * (26. + 22. * 1.5) * 8. / 10.) as Int,
            angle: g.angle,
        }
    }

    /// Ray direction of sub-cell `(gx, gy)`, unnormalized.
    #[inline(always)]
    fn dir(&self, gx: Float, gy: Float) -> [Float; 3] {
        let (vx, vy, v0) = (self.vx, self.vy, self.v0);
        [
            vx[0] * gx + vy[0] * gy + v0[0],
            vx[1] * gx + vy[1] * gy + v0[1],
            vx[2] * gx + vy[2] * gy + v0[2],
        ]
    }

    /// `(|d|^2, d . o, |d|^2 * discriminant)`. The scaled discriminant
    /// keeps every sign test exact without normalizing the ray: three
    /// multiplies where `normalize` costs a square root and three divides.
    #[inline(always)]
    fn geom(&self, d: &[Float; 3]) -> (Float, Float, Float) {
        let (o, oo, r2) = (self.o, self.oo, self.r2);
        let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        let d_o = d[0] * o[0] + d[1] * o[1] + d[2] * o[2];
        (l2, d_o, d_o * d_o - l2 * (oo - r2))
    }

    /// Blended day/night texture level at a surface point.
    #[inline(always)]
    fn level(&self, p: &[Float; 3]) -> Float {
        let (ex, ey) = self.texel(p);
        // in range by construction: texel() clamps to the texture size
        let i = ey * self.tex.size.0 + ex;
        let day = self.tex.day[i] as Float;
        match (&self.tex.night, self.night) {
            (Some(night), true) => {
                // luminance: dot(surface normal, light). The sun sits a
                // million radii away, so the light direction from any
                // surface point is SUN to within a millionth, and no
                // normalize is needed for it.
                let lum = (5.0 * (p[0] * SUN[0] + p[1] * SUN[1] + p[2] * SUN[2]) * self.inv_r
                    + 0.5)
                    .clamp(0., 1.);
                let n = night[i] as Float;
                ((1.0 - lum) * n + lum * day).min(self.max_level)
            }
            _ => day,
        }
    }

    /// Texel coordinates of a surface point.
    #[inline(always)]
    fn texel(&self, p: &[Float; 3]) -> (usize, usize) {
        let (tex_w, tex_h) = self.tex.size;
        let phi = (-p[2] * self.inv_r * 0.5 + 0.5).clamp(0.0, 1.0);
        let mut theta = p[1].atan2(p[0]) / (2. * PI) + 0.5 + self.angle / 2. / PI;
        theta -= theta.floor();
        let ex = ((theta * tex_w as Float) as usize).min(tex_w - 1);
        let ey = ((phi * tex_h as Float) as usize).min(tex_h - 1);
        (ex, ey)
    }

    /// Legacy ascii cell: one sample, palette glyph. Returns the glyph and
    /// whether it is a sky value the crossfade walks toward.
    #[inline]
    fn ascii_cell(&self, gx: Float, gy: Float) -> (char, bool) {
        let palette = &self.tex.palette;
        let mut u = self.dir(gx, gy);
        normalize(&mut u);
        let (ox, oy, oz) = (self.o[0], self.o[1], self.o[2]);
        let dot_uo = u[0] * ox + u[1] * oy + u[2] * oz;
        let discriminant = dot_uo * dot_uo - self.oo + self.r2;

        if discriminant < 0. {
            // analytic edge fringe (sub-cell coverage): a near-miss ray
            // still names a limb point (closest approach pushed onto the
            // sphere). Sample the surface there and thin its palette index
            // by coverage instead of hard-clipping to sky: the silhouette
            // thins down the ramp, 1-cell analytic antialias, crisp at any
            // font size. closest^2 = r^2 - discriminant; t = -dot_uo > 0
            // faces the globe.
            let t = -dot_uo;
            if t > 0. {
                let miss = (self.r * self.r - discriminant).sqrt() - self.r;
                // one *cell* steps u by ~1/(2 * half_height) of the cell
                // grid, times the limb range t.
                let foot = t * self.sy as Float / (2. * self.half_gh);
                if miss < foot {
                    let coverage = (1. - miss / foot).clamp(0., 1.);
                    let mut p = [ox + t * u[0], oy + t * u[1], oz + t * u[2]];
                    normalize(&mut p);
                    let (ex, ey) = self.unit_texel(&p);
                    let i = ey * self.tex.size.0 + ex;
                    let idx = *self.tex.day.get(i).unwrap_or(&0) as Float;
                    let thin = ((idx * coverage) as usize).min(palette.len() - 1);
                    return (palette[thin], false);
                }
            }
            return (sky_ascii(&u, self.qx, self.qz), true);
        }

        // globe surface: single centre sample, direct write
        let distance = -discriminant.sqrt() - dot_uo;
        let p = [
            ox + distance * u[0],
            oy + distance * u[1],
            oz + distance * u[2],
        ];
        let level = self.level(&p);
        (palette[(level as usize).min(palette.len() - 1)], false)
    }

    /// Texel coordinates of a point on the unit sphere (legacy fringe).
    #[inline]
    fn unit_texel(&self, p: &[Float; 3]) -> (usize, usize) {
        let (tex_w, tex_h) = self.tex.size;
        let phi = (-p[2] * 0.5 + 0.5).clamp(0.0, 1.0);
        let mut theta = p[1].atan2(p[0]) / (2. * PI) + 0.5 + self.angle / 2. / PI;
        theta -= theta.floor();
        let ex = ((theta * tex_w as Float) as usize).min(tex_w - 1);
        let ey = ((phi * tex_h as Float) as usize).min(tex_h - 1);
        (ex, ey)
    }

    /// Block cell: `sx x sy` samples, each with its own ray, dithered into
    /// one glyph. Silhouette, terminator, texture detail and star field all
    /// resolve at sub-cell resolution, which is what makes a small terminal
    /// font render a sharp globe instead of a coarse one.
    #[inline]
    fn block_cell(&self, cx: usize, cy: usize, gx0: Float, gy0: Float) -> char {
        let (sx, sy) = (self.sx, self.sy);
        let mut mask: u8 = 0;
        for iy in 0..sy {
            let y = (cy * sy + iy) & 7;
            let y0 = gy0 + iy as Float;
            for ix in 0..sx {
                let d = self.dir(gx0 + ix as Float, y0);
                if self.brightness(&d) > bayer(cx * sx + ix, y) {
                    mask |= BRAILLE_BITS[ix][iy];
                }
            }
        }
        glyph_for(self.glyph, mask)
    }

    /// Sub-cell sample brightness in `0.0 ..= 1.0`: globe surface when the
    /// ray hits, sky otherwise.
    #[inline(always)]
    fn brightness(&self, d: &[Float; 3]) -> Float {
        let (l2, d_o, disc) = self.geom(d);
        if disc < 0. {
            let inv = 1.0 / l2.sqrt();
            let u = [d[0] * inv, d[1] * inv, d[2] * inv];
            return sky_brightness(&u, self.qx, self.qz);
        }
        let t = (-d_o - disc.sqrt()) / l2;
        let o = self.o;
        let p = [o[0] + t * d[0], o[1] + t * d[1], o[2] + t * d[2]];
        self.level(&p) / self.max_level
    }
}

/// Bayer threshold of sub-cell `(x, y)`, in `0.0 .. 1.0`.
#[inline(always)]
fn bayer(x: usize, y: usize) -> Float {
    (BAYER8[(y & 7) * 8 + (x & 7)] as Float + 0.5) * (1.0 / 64.0)
}

/// Static star/band field hash: `(hash, depth, band distance)`. Stars hash
/// from the world-space ray plus the quantized camera, so camera drift pans
/// them and nothing pops frame to frame.
#[inline(always)]
fn sky_hash(u: &[Float; 3], qx: Int, qz: Int) -> (u32, u32, Float) {
    // parallax in world space: ray dir projected on the sky plane, scaled
    // per depth layer. stars are fixed in the sky, near layers pan faster.
    let ix = u[0] - SUN[0] * dot(u, &SUN);
    let iz = u[2] - SUN[2] * dot(u, &SUN);
    let mut h: u32 = ((u[1] * 997. + 0.5) as i32 as u32)
        .wrapping_mul(2246822519)
        .wrapping_add(
            ((u[0] * 571. + u[2] * 911. + 0.5) as i32 as u32).wrapping_mul(3266489917),
        );
    h = (h ^ (h >> 15)).wrapping_mul(2654435761);
    h ^= h >> 13;
    let depth = (h >> 27) & 3;
    let scl = 260. * (1 << depth) as Float;
    // camera-relative shift: rotate offset 90 deg from the sun axis so
    // orbit movement pans across the sky instead of into the pole.
    let sx = (ix * scl) as Int + qx * (1 + depth as Int) / 4;
    let sz = (iz * scl) as Int + qz * (1 + depth as Int) / 4;
    let mut sh: u32 = (sx as u32)
        .wrapping_mul(374761393)
        .wrapping_add((sz as u32).wrapping_mul(668265263))
        .wrapping_add(depth.wrapping_mul(2246822519));
    sh = (sh ^ (sh >> 13)).wrapping_mul(1274126177);
    sh ^= sh >> 16;
    let band_d = (u[0] * BAND[0] + u[1] * BAND[1] + u[2] * BAND[2]).abs();
    (sh, depth, band_d)
}

/// Milky way crossfade envelope at band distance `band_d`:
/// deep core 1.0 -> band edge ~0.0.
#[inline]
fn band_env(band_d: Float) -> Float {
    let g = (-(band_d * band_d) / (2. * CORE_W * CORE_W)).exp();
    let avg = (-(band_d * band_d) / (2. * OUT_W * OUT_W)).exp();
    g * 0.8 + avg * 0.2
}

/// Sky glyph for the ascii alphabet: sun disk, then stars, then milky way.
#[inline]
fn sky_ascii(u: &[Float; 3], qx: Int, qz: Int) -> char {
    let (sh, depth, band_d) = sky_hash(u, qx, qz);
    let in_band = band_d < IN_BAND;
    let core = band_d < CORE;
    // great rift: dark lane through the core, keeps some stars
    if in_band && core && band_d < DUST_W {
        let r = sh % 1000;
        if r % 10 < 7 {
            // dust blocks glow, sparse faint stars only
            return if r < 90 { '.' } else { ' ' };
        }
        return ' ';
    }
    // sun: ray-facing test around a fixed world direction
    let facing = u[0] * SUN[0] + u[1] * SUN[1] + u[2] * SUN[2];
    if facing > 0.99955 {
        return '*';
    }
    if facing > 0.99860 {
        let d = (facing - 0.99860) / 0.00095;
        return RAMP[(d * 6.) as usize];
    }
    if facing > 0.99630 {
        return '.';
    }
    if in_band {
        let r = sh % 1000;
        // hash picks a static tier 0..6, the envelope scales it, so faint
        // stays faint even in the core: no solid wall.
        let pick = (sh >> 9) % 7;
        let mut idx = (pick as Float * band_env(band_d)) as usize;
        if idx > 6 {
            idx = 6;
        }
        if core && idx == 5 && (sh % 13) == 0 {
            idx = 6; // rare static core star
        }
        let fill = if core { 720 } else { 200 };
        if r < fill {
            return RAMP[idx];
        }
        return ' ';
    }
    if sh % 1000 < 30 + depth * 8 {
        // sparse field ~3-5%: dots dominate, star rare
        return match (sh >> 24) % 13 {
            0..=7 => '.',
            8 | 9 => ':',
            10 => ';',
            11 => '+',
            _ => {
                if depth < 2 {
                    '*'
                } else {
                    ':'
                }
            }
        };
    }
    ' '
}

/// Sky brightness for the block alphabets: the same field as
/// [`sky_ascii`], as the value a dot thresholds against. Stars sit high on
/// the ramp so a single dot lights up instead of smearing over a cell.
#[inline]
fn sky_brightness(u: &[Float; 3], qx: Int, qz: Int) -> Float {
    let (sh, depth, band_d) = sky_hash(u, qx, qz);
    let in_band = band_d < IN_BAND;
    let core = band_d < CORE;
    if in_band && core && band_d < DUST_W {
        let r = sh % 1000;
        if r % 10 < 7 {
            return if r < 90 { 0.2 } else { 0.0 };
        }
        return 0.0;
    }
    let facing = u[0] * SUN[0] + u[1] * SUN[1] + u[2] * SUN[2];
    if facing > 0.99955 {
        return 1.0;
    }
    if facing > 0.99860 {
        return ((facing - 0.99860) / 0.00095).clamp(0., 1.);
    }
    if facing > 0.99630 {
        return 0.3;
    }
    if in_band {
        let r = sh % 1000;
        let pick = (sh >> 9) % 7;
        let mut idx = (pick as Float * band_env(band_d)) as usize;
        if idx > 6 {
            idx = 6;
        }
        if core && idx == 5 && (sh % 13) == 0 {
            idx = 6;
        }
        let fill = if core { 720 } else { 200 };
        if r < fill {
            return idx as Float / 6.;
        }
        return 0.0;
    }
    if sh % 1000 < 30 + depth * 8 {
        return match (sh >> 24) % 13 {
            0..=7 => 0.7,
            8 | 9 => 0.8,
            10 => 0.9,
            11 => 0.95,
            _ => 1.0,
        };
    }
    0.0
}

/// One texture source in a [`GlobeConfig`].
#[derive(Clone)]
enum ImageSource {
    /// Ascii image, indexed when the globe is built.
    Ascii(String),
    /// Baked image, used in place.
    Baked(Baked),
}

/// Globe configuration struct implementing the builder pattern.
#[derive(Default)]
pub struct GlobeConfig {
    camera_cfg: Option<CameraConfig>,
    radius: Option<Float>,
    angle: Option<Float>,
    template: Option<GlobeTemplate>,
    day: Option<ImageSource>,
    night: Option<ImageSource>,
    palette: Option<Vec<char>>,
    display_night: bool,
    glyph: Option<Glyph>,
}

impl GlobeConfig {
    /// Creates an empty `GlobeConfig`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets `CameraConfig` to be used by the builder.
    pub fn with_camera(mut self, config: CameraConfig) -> Self {
        self.camera_cfg = Some(config);
        self
    }

    /// Sets the globe radius.
    pub fn with_radius(mut self, r: Float) -> Self {
        self.radius = Some(r);
        self
    }

    /// Selects a template to be used by the builder.
    pub fn use_template(mut self, t: GlobeTemplate) -> Self {
        self.template = Some(t);
        self
    }

    /// Sets the sub-cell glyph alphabet, see [`Glyph`].
    pub fn with_glyph(mut self, glyph: Glyph) -> Self {
        self.glyph = Some(glyph);
        self
    }

    /// Sets the day texture to be displayed on the globe.
    pub fn with_texture(mut self, texture: &str, palette: Option<Vec<char>>) -> Self {
        self.day = Some(ImageSource::Ascii(texture.to_string()));
        if palette.is_some() {
            self.palette = palette;
        }
        self
    }

    /// Sets the night texture to be displayed on the globe.
    pub fn with_night_texture(mut self, texture: &str, palette: Option<Vec<char>>) -> Self {
        self.night = Some(ImageSource::Ascii(texture.to_string()));
        if palette.is_some() {
            self.palette = palette;
        }
        self
    }

    /// Sets the day texture to be loaded from the given path.
    pub fn with_texture_at(self, path: &str, palette: Option<Vec<char>>) -> Self {
        let mut file = File::open(path).unwrap();
        let mut out_string = String::new();
        file.read_to_string(&mut out_string).unwrap();
        self.with_texture(&out_string, palette)
    }

    /// Sets the night texture to be loaded from the given path.
    pub fn with_night_texture_at(self, path: &str, palette: Option<Vec<char>>) -> Self {
        let mut file = File::open(path).unwrap();
        let mut out_string = String::new();
        file.read_to_string(&mut out_string).unwrap();
        self.with_night_texture(&out_string, palette)
    }

    /// Sets the day texture from a baked image.
    pub fn with_baked_texture(mut self, day: Baked) -> Self {
        self.day = Some(ImageSource::Baked(day));
        self
    }

    /// Sets the night texture from a baked image.
    pub fn with_night_baked_texture(mut self, night: Baked) -> Self {
        self.night = Some(ImageSource::Baked(night));
        self
    }

    /// Sets the night display toggle to the given value.
    pub fn display_night(mut self, b: bool) -> Self {
        self.display_night = b;
        self
    }

    /// Builds new `Globe` from the collected configuration settings.
    pub fn build(mut self) -> Globe {
        if let Some(template) = self.template {
            // An explicit day map replaces the whole template: blending a
            // custom map with a built-in night side would mean mixing two
            // unrelated palettes and sizes.
            if self.day.is_none() {
                let (day, night) = template.maps();
                self.day = Some(ImageSource::Baked(Baked::parse(day)));
                if self.night.is_none() {
                    self.night = night.map(|n| ImageSource::Baked(Baked::parse(n)));
                }
            }
        }
        let texture = match (self.day.take(), self.night.take()) {
            (Some(day), night) => assemble(day, night, self.palette.take()),
            // night only: the day side shows the same image
            (None, Some(night)) => assemble(night.clone(), Some(night), self.palette.take()),
            (None, None) => panic!("texture not provided"),
        };
        let camera = self
            .camera_cfg
            .unwrap_or_else(CameraConfig::default)
            .build();
        Globe {
            camera,
            radius: self.radius.unwrap_or(1.),
            angle: self.angle.unwrap_or(0.),
            texture,
            display_night: self.display_night,
            frame: 0,
            glyph: self.glyph.unwrap_or(Glyph::Braille),
        }
    }
}

/// Builds a texture from its day and night images. Ascii images share one
/// palette; a baked night image must arrive with the same palette and size.
fn assemble(day: ImageSource, night: Option<ImageSource>, palette: Option<Vec<char>>) -> Texture {
    let (day, mut palette, size) = match day {
        ImageSource::Ascii(image) => {
            let mut palette = palette.unwrap_or_default();
            let (data, size) = index_image(&image, &mut palette);
            (Cow::Owned(data), palette, size)
        }
        ImageSource::Baked(baked) => (Cow::Borrowed(baked.data), baked.palette.clone(), baked.size),
    };
    let night = night.map(|night| match night {
        ImageSource::Ascii(image) => {
            let (data, night_size) = index_image(&image, &mut palette);
            assert_eq!(
                night_size, size,
                "night texture must match the day texture size"
            );
            Cow::Owned(data)
        }
        ImageSource::Baked(baked) => {
            assert_eq!(
                baked.palette, palette,
                "day and night textures need the same palette"
            );
            assert_eq!(
                baked.size, size,
                "day and night textures need the same size"
            );
            Cow::Borrowed(baked.data)
        }
    });
    Texture {
        day,
        night,
        palette,
        size,
    }
}

/// Built-in globe template: one solar system body.
///
/// Each template carries a baked map at 1440x720 palette levels; earth also
/// carries a night side. Bodies without a night map simply ignore
/// [`GlobeConfig::display_night`], which is also what makes the sun
/// emissive: no night map, no terminator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlobeTemplate {
    Earth,
    Sun,
    Mercury,
    Venus,
    Moon,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl GlobeTemplate {
    /// Every template, in menu order.
    pub const ALL: [GlobeTemplate; 10] = [
        GlobeTemplate::Earth,
        GlobeTemplate::Sun,
        GlobeTemplate::Mercury,
        GlobeTemplate::Venus,
        GlobeTemplate::Moon,
        GlobeTemplate::Mars,
        GlobeTemplate::Jupiter,
        GlobeTemplate::Saturn,
        GlobeTemplate::Uranus,
        GlobeTemplate::Neptune,
    ];

    /// Stable lowercase name, as accepted by [`GlobeTemplate::from_name`].
    pub const fn name(self) -> &'static str {
        match self {
            GlobeTemplate::Earth => "earth",
            GlobeTemplate::Sun => "sun",
            GlobeTemplate::Mercury => "mercury",
            GlobeTemplate::Venus => "venus",
            GlobeTemplate::Moon => "moon",
            GlobeTemplate::Mars => "mars",
            GlobeTemplate::Jupiter => "jupiter",
            GlobeTemplate::Saturn => "saturn",
            GlobeTemplate::Uranus => "uranus",
            GlobeTemplate::Neptune => "neptune",
        }
    }

    /// Names of every template, for menus and CLI validation.
    pub const NAMES: [&'static str; GlobeTemplate::ALL.len()] = {
        let mut names = [""; GlobeTemplate::ALL.len()];
        let mut i = 0;
        while i < GlobeTemplate::ALL.len() {
            names[i] = GlobeTemplate::ALL[i].name();
            i += 1;
        }
        names
    };

    /// Parses a [`GlobeTemplate::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|t| t.name() == name)
    }

    /// Baked day map, plus the night map for bodies that have one.
    fn maps(self) -> (&'static [u8], Option<&'static [u8]>) {
        match self {
            GlobeTemplate::Earth => (EARTH_HD, Some(EARTH_NIGHT_HD)),
            GlobeTemplate::Sun => (SUN_HD, None),
            GlobeTemplate::Mercury => (MERCURY_HD, None),
            GlobeTemplate::Venus => (VENUS_HD, None),
            GlobeTemplate::Moon => (MOON_HD, None),
            GlobeTemplate::Mars => (MARS_HD, None),
            GlobeTemplate::Jupiter => (JUPITER_HD, None),
            GlobeTemplate::Saturn => (SATURN_HD, None),
            GlobeTemplate::Uranus => (URANUS_HD, None),
            GlobeTemplate::Neptune => (NEPTUNE_HD, None),
        }
    }
}

static EARTH_HD: &[u8] = include_bytes!("../textures/earth_hd.gidx");
static EARTH_NIGHT_HD: &[u8] = include_bytes!("../textures/earth_night_hd.gidx");
static SUN_HD: &[u8] = include_bytes!("../textures/sun_hd.gidx");
static MERCURY_HD: &[u8] = include_bytes!("../textures/mercury_hd.gidx");
static VENUS_HD: &[u8] = include_bytes!("../textures/venus_hd.gidx");
static MOON_HD: &[u8] = include_bytes!("../textures/moon_hd.gidx");
static MARS_HD: &[u8] = include_bytes!("../textures/mars_hd.gidx");
static JUPITER_HD: &[u8] = include_bytes!("../textures/jupiter_hd.gidx");
static SATURN_HD: &[u8] = include_bytes!("../textures/saturn_hd.gidx");
static URANUS_HD: &[u8] = include_bytes!("../textures/uranus_hd.gidx");
static NEPTUNE_HD: &[u8] = include_bytes!("../textures/neptune_hd.gidx");

/// Camera configuration struct implementing the builder pattern.
pub struct CameraConfig {
    radius: Float,
    alpha: Float,
    beta: Float,
}

impl CameraConfig {
    /// Creates a new `CameraConfig`.
    ///
    /// # Arguments
    ///
    /// - `r` is the distance from the camera to the origin.
    /// - `alfa` is camera's angle along the xy plane.
    /// - `beta` is camera's angle along z axis.
    pub fn new(radius: Float, alpha: Float, beta: Float) -> Self {
        Self {
            radius,
            alpha,
            beta,
        }
    }

    /// Creates a new `CameraConfig` using default values.
    pub fn default() -> Self {
        Self {
            radius: 2.,
            alpha: 0.,
            beta: 0.,
        }
    }

    /// Builds a camera from the collected config information.
    pub fn build(&self) -> Camera {
        let mut camera = Camera::default();
        camera.update(self.radius, self.alpha, self.beta);
        camera
    }
}

#[derive(Default)]
pub struct Camera {
    x: Float,
    y: Float,
    z: Float,
    matrix: [Float; 16],
}

impl Camera {
    /// Updates the camera using new data.
    pub fn update(&mut self, r: Float, alpha: Float, beta: Float) {
        let sin_a = alpha.sin();
        let cos_a = alpha.cos();
        let sin_b = beta.sin();
        let cos_b = beta.cos();

        let x = r * cos_a * cos_b;
        let y = r * sin_a * cos_b;
        let z = r * sin_b;

        let mut matrix = [0.; 16];

        // matrix
        matrix[3] = 0.;
        matrix[7] = 0.;
        matrix[11] = 0.;
        matrix[15] = 1.;
        // x
        matrix[0] = -sin_a;
        matrix[1] = cos_a;
        matrix[2] = 0.;
        // y
        matrix[4] = cos_a * sin_b;
        matrix[5] = sin_a * sin_b;
        matrix[6] = -cos_b;
        // z
        matrix[8] = cos_a * cos_b;
        matrix[9] = sin_a * cos_b;
        matrix[10] = sin_b;

        matrix[12] = x;
        matrix[13] = y;
        matrix[14] = z;

        self.x = x;
        self.y = y;
        self.z = z;
        self.matrix = matrix;
    }
}

/// Get index of the given character on the palette.
fn find_index(target: char, palette: &[char]) -> Int {
    for (i, &ch) in palette.iter().enumerate() {
        if target == ch {
            return i as Int;
        }
    }
    -1
}

fn dot(a: &[Float; 3], b: &[Float; 3]) -> Float {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn magnitude(r: &[Float; 3]) -> Float {
    dot(r, r).sqrt()
}

fn normalize(r: &mut [Float; 3]) {
    let len: Float = magnitude(r);
    r[0] /= len;
    r[1] /= len;
    r[2] /= len;
}
