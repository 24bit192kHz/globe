//! Customizable ASCII globe generator.
//!
//! Based on [C++ code by DinoZ1729](https://github.com/DinoZ1729/Earth).

#![allow(dead_code)]

use std::f32::consts::PI;
use std::fs::File;
use std::io::Read;

pub type Int = isize;
pub type Float = f32;

/// Sky brightness ramp, dim -> bright. Crossfade walks this one step
/// per frame toward target: ultra-smooth, never pops or flashes.
const RAMP: [char; 7] = [' ', '.', '.', ':', ';', '+', '*'];

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

static EARTH_TEXTURE: &str = include_str!("../textures/earth_hd.txt");
static EARTH_NIGHT_TEXTURE: &str = include_str!("../textures/earth_night_hd.txt");

/// Globe texture.
pub struct Texture {
    day: Vec<Vec<char>>,
    night: Option<Vec<Vec<char>>>,
    palette: Option<Vec<char>>,
}

impl Texture {
    pub fn new(
        day: Vec<Vec<char>>,
        night: Option<Vec<Vec<char>>>,
        palette: Option<Vec<char>>,
    ) -> Self {
        Texture {
            day,
            night,
            palette,
        }
    }
    pub fn get_size(&self) -> (usize, usize) {
        (self.day[0].len() - 1, self.day.len() - 1)
    }
}

/// Canvas that will be used to render the globe onto.
pub struct Canvas {
    pub matrix: Vec<Vec<char>>,
    size: (usize, usize),
    // character size
    pub char_pix: (usize, usize),
}

impl Canvas {
    pub fn new(x: u16, y: u16, cp: Option<(usize, usize)>) -> Self {
        let x = x as usize;
        let y = y as usize;

        let matrix = vec![vec![' '; x]; y];

        Self {
            size: (x, y),
            matrix,
            char_pix: cp.unwrap_or((4, 8)),
        }
    }
    pub fn get_size(&self) -> (usize, usize) {
        self.size
    }
    pub fn clear(&mut self) {
        // only displayed cells ever printed (32x cheaper than full matrix)
        let dw = self.size.0 / self.char_pix.0;
        let dh = self.size.1 / self.char_pix.1;
        for row in self.matrix.iter_mut().take(dh) {
            for c in row.iter_mut().take(dw) {
                *c = ' ';
            }
        }
    }
    fn draw_point(&mut self, a: usize, b: usize, c: char) {
        if a >= self.size.0 || b >= self.size.1 {
            return;
        }
        self.matrix[b][a] = c;
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
}

impl Globe {
    pub fn render_on(&mut self, canvas: &mut Canvas) {
        self.frame = self.frame.wrapping_add(1);
        // sun sits ~41 deg off launch view axis: visible beside earth,
        // outside globe disk. light matches sun dir so lit side coherent.
        // unit length: |v| = 1.0
        const SUN: [Float; 3] = [-0.7547, 0.5535, 0.3523];
        // let there be light
        let light: [Float; 3] = [SUN[0] * 999999., SUN[1] * 999999., SUN[2] * 999999.];
        const BAND: [Float; 3] = [0.399, 0.349, 0.848];
        // shoot one ray per *displayed* cell (char resolution:
        // 32x fewer rays than pixel resolution, identical visible output)
        let (size_x, size_y) = canvas.get_size();
        let dw = size_x / canvas.char_pix.0;
        let dh = size_y / canvas.char_pix.1;
        let half_w = dw as Int / 2;
        let half_h = dh as Int / 2;
        let (ox, oy, oz) = (self.camera.x, self.camera.y, self.camera.z);
        let m = self.camera.matrix;
        // quantize camera shift to 1/8-cell bins: background stable
        // for many frames, then pans. crossfade below smooths the step.
        // stars damped 10x vs earth: tiny fraction of camera travel.
        let qx = (ox * (26. + 22. * 1.5) * 8. / 10.) as Int;
        let qz = (oz * (26. + 22. * 1.5) * 8. / 10.) as Int;
        for yi in 0..dh {
            let yif = yi as Int;
            for xi in 0..dw {
                let xif = xi as Int;
                // coordinates of the camera, origin of the ray
                let o: [Float; 3] = [ox, oy, oz];
                // x normalized by 2*half_h (not half_w): terminal cells
                // are ~2:1 tall, this keeps the globe circular fullscreen
                let mut u: [Float; 3] = [
                    -((xif - half_w) as Float + 0.5) / (2 * half_h) as Float,
                    ((yif - half_h) as Float + 0.5) / half_h as Float,
                    -1.,
                ];
                transform_vector(&mut u, m);
                u[0] -= ox;
                u[1] -= oy;
                u[2] -= oz;
                normalize(&mut u);
                let dot_uo = dot(&u, &o);
                let discriminant: Float = dot_uo * dot_uo - dot(&o, &o) + self.radius * self.radius;

                // target char for this cell; crossfade walks current -> target
                let mut target = ' ';
                // ray misses globe: sun disk > parallax stars > milkyway dust
                if discriminant < 0. {
                    // analytic edge fringe (sub-cell coverage): a near-miss
                    // ray still names a limb point (closest approach pushed
                    // onto the sphere). sample the surface there and thin
                    // its palette index by coverage instead of hard-clipping
                    // to sky: the silhouette thins down the ramp, 1-cell
                    // analytic antialias, crisp at any font size.
                    // closest^2 = r^2 - discriminant; t = -dot_uo > 0 faces globe.
                    let t = -dot_uo;
                    if t > 0. {
                        let miss =
                            (self.radius * self.radius - discriminant).sqrt() - self.radius;
                        // world-space ray footprint at limb range: one cell
                        // steps u by ~1/(2*half_h), times range t.
                        let foot = t / (2 * half_h) as Float;
                        if miss < foot {
                            if let Some(palette) = self.texture.palette.as_ref() {
                                let coverage = (1. - miss / foot).clamp(0., 1.);
                                let mut p =
                                    [ox + t * u[0], oy + t * u[1], oz + t * u[2]];
                                normalize(&mut p);
                                let phi = (-p[2] * 0.5 + 0.5).clamp(0.0, 1.0);
                                let mut theta = p[1].atan2(p[0]) / (2. * PI)
                                    + 0.5
                                    + self.angle / 2. / PI;
                                theta -= theta.floor();
                                let w = self.texture.day[0].len();
                                let h = self.texture.day.len();
                                let mut ex = (theta * w as Float) as usize;
                                if ex >= w {
                                    ex = w - 1;
                                }
                                let mut ey = (phi * h as Float) as usize;
                                if ey >= h {
                                    ey = h - 1;
                                }
                                let idx =
                                    find_index(self.texture.day[ey][ex], palette);
                                if idx >= 0 {
                                    let thin = ((idx as Float * coverage) as usize)
                                        .min(palette.len() - 1);
                                    canvas.matrix[yi][xi] = palette[thin];
                                    continue;
                                }
                            }
                        }
                    }
                    // parallax in world space: ray dir projected on sky plane,
                    // scaled per depth layer, drifted by quantized camera.
                    // stars fixed in sky, near layers pan faster.
                    let ix = u[0] - SUN[0] * dot(&u, &SUN);
                    let iz = u[2] - SUN[2] * dot(&u, &SUN);
                    let mut h: u32 = ((u[1] * 997. + 0.5) as i32 as u32)
                        .wrapping_mul(2246822519)
                        .wrapping_add(
                            ((u[0] * 571. + u[2] * 911. + 0.5) as i32 as u32)
                                .wrapping_mul(3266489917),
                        );
                    h = (h ^ (h >> 15)).wrapping_mul(2654435761);
                    h ^= h >> 13;
                    let depth = (h >> 27) & 3;
                    let scl = 260. * (1 << depth) as Float;
                    // camera-relative shift: rotate offset by 90 deg from sun
                    // axis so orbit movement pans across sky, not into pole
                    let sx = (ix * scl) as Int + qx * (1 + depth as Int) / 4;
                    let sz = (iz * scl) as Int + qz * (1 + depth as Int) / 4;
                    let mut sh: u32 = (sx as u32)
                        .wrapping_mul(374761393)
                        .wrapping_add((sz as u32).wrapping_mul(668265263))
                        .wrapping_add(depth.wrapping_mul(2246822519));
                    sh = (sh ^ (sh >> 13)).wrapping_mul(1274126177);
                    sh ^= sh >> 16;
                    // fixed world-space band, camera-independent
                    let bs = u[0] * BAND[0] + u[1] * BAND[1] + u[2] * BAND[2];
                    let band_d = bs.abs();
                    let in_band = band_d < 0.16;
                    let core = band_d < 0.06;
                    // static sky: zero twinkle = zero flash, near-zero redraw.
                    // stars hash from world-space ray + quantized camera;
                    // camera drift pans them, nothing pops frame to frame.
                    // crossfade: quantized camera bins blend old char -> new
                    // char one ramp step per frame = ultra-smooth, no flash.
                    let core_w = 0.06; // bright core half-width (~7deg full)
                    let out_w = 0.16; // faint band edge (~18deg full)
                    let dust_w = 0.016; // great rift half-width
                    let g = (-(band_d * band_d) / (2. * core_w * core_w)).exp();
                    let avg = (-(band_d * band_d) / (2. * out_w * out_w)).exp();
                    // great rift: dark lane through core, keep some stars
                    if in_band && core && band_d < dust_w {
                        let r = sh % 1000;
                        if r % 10 < 7 {
                            // dust blocks glow, sparse faint stars only
                            if r < 90 {
                                target = '.';
                            }
                        }
                    } else {
                        // sun: ray-facing test around fixed world direction
                        let facing = u[0] * SUN[0] + u[1] * SUN[1] + u[2] * SUN[2];
                        if facing > 0.99955 {
                            target = '*';
                        } else if facing > 0.99860 {
                            let d = (facing - 0.99860) / 0.00095;
                            target = RAMP[(d * 6.) as usize];
                        } else if facing > 0.99630 {
                            target = '.';
                        } else if in_band {
                            let r = sh % 1000;
                            // crossfade envelope: deep core 1.0 -> edge ~0.0.
                            // hash picks static tier 0..6, envelope scales it.
                            // faint stays faint even in core: no solid wall.
                            let env = g * 0.8 + avg * 0.2;
                            let pick = (sh >> 9) % 7;
                            let mut idx = (pick as Float * env) as usize;
                            if idx > 6 {
                                idx = 6;
                            }
                            if core && idx == 5 && (sh % 13) == 0 {
                                idx = 6; // rare static core star
                            }
                            let fill = if core { 720 } else { 200 };
                            if r < fill {
                                target = RAMP[idx];
                            }
                        } else if sh % 1000 < 30 + depth * 8 {
                            // sparse field ~3-5%: dots dominate, star rare
                            target = match (sh >> 24) % 13 {
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
                    }
                    // crossfade: walk current cell one ramp step toward
                    // target per frame. chars appear/disappear as
                    // ' ' -> '.' -> ':' -> ';' -> '+' -> '*' = diffuse,
                    // ultra-smooth, zero flash. globe chars pass through.
                    let cur = canvas.matrix[yi][xi];
                    if cur == target {
                        continue;
                    }
                    match (ramp_idx(cur), ramp_idx(target)) {
                        (Some(a), Some(b)) => {
                            canvas.matrix[yi][xi] = RAMP[a + (b > a) as usize
                                - (b < a) as usize];
                        }
                        _ => {
                            // entering sky from globe char: start diffuse
                            canvas.matrix[yi][xi] = if target == ' ' {
                                ' '
                            } else if target == '*' {
                                '+'
                            } else {
                                '.'
                            };
                        }
                    }
                    continue;
                }

                // globe surface: single center sample, direct write.
                let distance: Float = -discriminant.sqrt() - dot_uo;

                // intersection point
                let inter: [Float; 3] = [
                    ox + distance * u[0],
                    oy + distance * u[1],
                    oz + distance * u[2],
                ];

                // surface normal
                let mut n: [Float; 3] = [
                    ox + distance * u[0],
                    oy + distance * u[1],
                    oz + distance * u[2],
                ];
                normalize(&mut n);

                // unit vector pointing from intersection to light source
                let mut l: [Float; 3] = [0.; 3];
                vector(&mut l, &inter, &light);
                normalize(&mut l);
                let luminance: Float = clamp(5. * (dot(&n, &l)) + 0.5, 0., 1.);

                // computing coordinates for the sphere.
                // atan2 = true longitude (single wrap, no seam). old
                // atan(y/x) mirrored a hemisphere and doubled the map,
                // which blew up into radial streaks near the poles.
                // nearest-neighbor on purpose: palette indices are
                // categorical (land/ocean glyphs), blending them invents
                // mid-index letters = blocky halo artifacts at coasts.
                let phi = (-inter[2] / self.radius * 0.5 + 0.5).clamp(0.0, 1.0);
                let mut theta =
                    inter[1].atan2(inter[0]) / (2. * PI) + 0.5 + self.angle / 2. / PI;
                theta -= theta.floor();
                let w = self.texture.day[0].len();
                let h = self.texture.day.len();
                let mut ex = (theta * w as Float) as usize;
                if ex >= w {
                    ex = w - 1;
                }
                let mut ey = (phi * h as Float) as usize;
                if ey >= h {
                    ey = h - 1;
                }
                let earth_x = ex;
                let earth_y = ey;

                let ch = if self.display_night
                    && self.texture.night.is_some()
                    && self.texture.palette.is_some()
                {
                    let palette = self.texture.palette.as_ref().unwrap();
                    let day = find_index(self.texture.day[earth_y][earth_x], palette);
                    let night = find_index(
                        self.texture.night.as_ref().unwrap()[earth_y][earth_x],
                        palette,
                    );

                    let mut index =
                        ((1.0 - luminance) * night as Float + luminance * day as Float) as usize;
                    if index >= palette.len() {
                        index = 0;
                    }
                    palette[index]
                }
                // else just draw the day texture without considering luminance
                else {
                    self.texture.day[earth_y][earth_x]
                };
                canvas.matrix[yi][xi] = ch;
            }
        }
    }
}

/// Globe configuration struct implementing the builder pattern.
#[derive(Default)]
pub struct GlobeConfig {
    camera_cfg: Option<CameraConfig>,
    radius: Option<Float>,
    angle: Option<Float>,
    template: Option<GlobeTemplate>,
    texture: Option<Texture>,
    display_night: bool,
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

    /// Sets the day texture to be displayed on the globe.
    pub fn with_texture(mut self, texture: &str, palette: Option<Vec<char>>) -> Self {
        let mut day = Vec::new();
        let lines = texture.lines();
        for line in lines {
            let row: Vec<char> = line.chars().rev().collect();
            day.push(row);
        }
        if let Some(texture) = &mut self.texture {
            texture.day = day;
        } else {
            self.texture = Some(Texture::new(day, None, palette));
        }
        self
    }

    /// Sets the night texture to be displayed on the globe.
    pub fn with_night_texture(mut self, texture: &str, palette: Option<Vec<char>>) -> Self {
        let mut night = Vec::new();
        let lines = texture.lines();
        for line in lines {
            let row: Vec<char> = line.chars().rev().collect();
            night.push(row);
        }

        if let Some(texture) = &mut self.texture {
            texture.night = Some(night);
        } else {
            self.texture = Some(Texture::new(night.clone(), Some(night), palette));
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

    /// Sets the night display toggle to the given value.
    pub fn display_night(mut self, b: bool) -> Self {
        self.display_night = b;
        self
    }

    /// Builds new `Globe` from the collected configuration settings.
    pub fn build(mut self) -> Globe {
        if let Some(template) = &self.template {
            match template {
                GlobeTemplate::Earth => {
                    let palette = vec![
                        ' ', '.', ':', ';', '\'', ',', 'w', 'i', 'o', 'g', 'O', 'L', 'X', 'H', 'W',
                        'Y', 'V', '@',
                    ];
                    self = self
                        .with_texture(EARTH_TEXTURE, Some(palette.clone()))
                        .with_night_texture(EARTH_NIGHT_TEXTURE, Some(palette))
                }
            }
        }
        let texture = self.texture.expect("texture not provided");
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
        }
    }
}

/// Built-in globe template enumeration.
pub enum GlobeTemplate {
    Earth,
    // Moon,
    // Mars,
}

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
    inv: [Float; 16],
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

        let mut inv = [0.; 16];
        invert(&mut inv, matrix);

        self.x = x;
        self.y = y;
        self.z = z;
        self.matrix = matrix;
        self.inv = inv;
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

fn transform_vector(vec: &mut [Float; 3], m: [Float; 16]) {
    let tx: Float = vec[0] * m[0] + vec[1] * m[4] + vec[2] * m[8] + m[12];
    let ty: Float = vec[0] * m[1] + vec[1] * m[5] + vec[2] * m[9] + m[13];
    let tz: Float = vec[0] * m[2] + vec[1] * m[6] + vec[2] * m[10] + m[14];
    vec[0] = tx;
    vec[1] = ty;
    vec[2] = tz;
}

fn invert(inv: &mut [Float; 16], matrix: [Float; 16]) {
    inv[0] = matrix[5] * matrix[10] * matrix[15]
        - matrix[5] * matrix[11] * matrix[14]
        - matrix[9] * matrix[6] * matrix[15]
        + matrix[9] * matrix[7] * matrix[14]
        + matrix[13] * matrix[6] * matrix[11]
        - matrix[13] * matrix[7] * matrix[10];

    inv[4] = -matrix[4] * matrix[10] * matrix[15]
        + matrix[4] * matrix[11] * matrix[14]
        + matrix[8] * matrix[6] * matrix[15]
        - matrix[8] * matrix[7] * matrix[14]
        - matrix[12] * matrix[6] * matrix[11]
        + matrix[12] * matrix[7] * matrix[10];

    inv[8] = matrix[4] * matrix[9] * matrix[15]
        - matrix[4] * matrix[11] * matrix[13]
        - matrix[8] * matrix[5] * matrix[15]
        + matrix[8] * matrix[7] * matrix[13]
        + matrix[12] * matrix[5] * matrix[11]
        - matrix[12] * matrix[7] * matrix[9];

    inv[12] = -matrix[4] * matrix[9] * matrix[14]
        + matrix[4] * matrix[10] * matrix[13]
        + matrix[8] * matrix[5] * matrix[14]
        - matrix[8] * matrix[6] * matrix[13]
        - matrix[12] * matrix[5] * matrix[10]
        + matrix[12] * matrix[6] * matrix[9];

    inv[1] = -matrix[1] * matrix[10] * matrix[15]
        + matrix[1] * matrix[11] * matrix[14]
        + matrix[9] * matrix[2] * matrix[15]
        - matrix[9] * matrix[3] * matrix[14]
        - matrix[13] * matrix[2] * matrix[11]
        + matrix[13] * matrix[3] * matrix[10];

    inv[5] = matrix[0] * matrix[10] * matrix[15]
        - matrix[0] * matrix[11] * matrix[14]
        - matrix[8] * matrix[2] * matrix[15]
        + matrix[8] * matrix[3] * matrix[14]
        + matrix[12] * matrix[2] * matrix[11]
        - matrix[12] * matrix[3] * matrix[10];

    inv[9] = -matrix[0] * matrix[9] * matrix[15]
        + matrix[0] * matrix[11] * matrix[13]
        + matrix[8] * matrix[1] * matrix[15]
        - matrix[8] * matrix[3] * matrix[13]
        - matrix[12] * matrix[1] * matrix[11]
        + matrix[12] * matrix[3] * matrix[9];

    inv[13] = matrix[0] * matrix[9] * matrix[14]
        - matrix[0] * matrix[10] * matrix[13]
        - matrix[8] * matrix[1] * matrix[14]
        + matrix[8] * matrix[2] * matrix[13]
        + matrix[12] * matrix[1] * matrix[10]
        - matrix[12] * matrix[2] * matrix[9];

    inv[2] = matrix[1] * matrix[6] * matrix[15]
        - matrix[1] * matrix[7] * matrix[14]
        - matrix[5] * matrix[2] * matrix[15]
        + matrix[5] * matrix[3] * matrix[14]
        + matrix[13] * matrix[2] * matrix[7]
        - matrix[13] * matrix[3] * matrix[6];

    inv[6] = -matrix[0] * matrix[6] * matrix[15]
        + matrix[0] * matrix[7] * matrix[14]
        + matrix[4] * matrix[2] * matrix[15]
        - matrix[4] * matrix[3] * matrix[14]
        - matrix[12] * matrix[2] * matrix[7]
        + matrix[12] * matrix[3] * matrix[6];

    inv[10] = matrix[0] * matrix[5] * matrix[15]
        - matrix[0] * matrix[7] * matrix[13]
        - matrix[4] * matrix[1] * matrix[15]
        + matrix[4] * matrix[3] * matrix[13]
        + matrix[12] * matrix[1] * matrix[7]
        - matrix[12] * matrix[3] * matrix[5];

    inv[14] = -matrix[0] * matrix[5] * matrix[14]
        + matrix[0] * matrix[6] * matrix[13]
        + matrix[4] * matrix[1] * matrix[14]
        - matrix[4] * matrix[2] * matrix[13]
        - matrix[12] * matrix[1] * matrix[6]
        + matrix[12] * matrix[2] * matrix[5];

    inv[3] = -matrix[1] * matrix[6] * matrix[11]
        + matrix[1] * matrix[7] * matrix[10]
        + matrix[5] * matrix[2] * matrix[11]
        - matrix[5] * matrix[3] * matrix[10]
        - matrix[9] * matrix[2] * matrix[7]
        + matrix[9] * matrix[3] * matrix[6];

    inv[7] = matrix[0] * matrix[6] * matrix[11]
        - matrix[0] * matrix[7] * matrix[10]
        - matrix[4] * matrix[2] * matrix[11]
        + matrix[4] * matrix[3] * matrix[10]
        + matrix[8] * matrix[2] * matrix[7]
        - matrix[8] * matrix[3] * matrix[6];

    inv[11] = -matrix[0] * matrix[5] * matrix[11]
        + matrix[0] * matrix[7] * matrix[9]
        + matrix[4] * matrix[1] * matrix[11]
        - matrix[4] * matrix[3] * matrix[9]
        - matrix[8] * matrix[1] * matrix[7]
        + matrix[8] * matrix[3] * matrix[5];

    inv[15] = matrix[0] * matrix[5] * matrix[10]
        - matrix[0] * matrix[6] * matrix[9]
        - matrix[4] * matrix[1] * matrix[10]
        + matrix[4] * matrix[2] * matrix[9]
        + matrix[8] * matrix[1] * matrix[6]
        - matrix[8] * matrix[2] * matrix[5];

    let mut det: Float =
        matrix[0] * inv[0] + matrix[1] * inv[4] + matrix[2] * inv[8] + matrix[3] * inv[12];

    det = 1.0 / det;

    for inv_i in inv.iter_mut() {
        *inv_i *= det;
    }
}

fn cross(r: &mut [Float; 3], a: [Float; 3], b: [Float; 3]) {
    r[0] = a[1] * b[2] - a[2] * b[1];
    r[1] = a[2] * b[0] - a[0] * b[2];
    r[2] = a[0] * b[1] - a[1] * b[0];
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

fn dot(a: &[Float; 3], b: &[Float; 3]) -> Float {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vector(a: &mut [Float; 3], b: &[Float; 3], c: &[Float; 3]) {
    a[0] = b[0] - c[0];
    a[1] = b[1] - c[1];
    a[2] = b[2] - c[2];
}

fn transform_vector2(vec: &mut [Float; 3], m: &[Float; 9]) {
    vec[0] = m[0] * vec[0] + m[1] * vec[1] + m[2] * vec[2];
    vec[1] = m[3] * vec[0] + m[4] * vec[1] + m[5] * vec[2];
    vec[2] = m[6] * vec[0] + m[7] * vec[1] + m[8] * vec[2];
}

fn rotate_x(vec: &mut [Float; 3], theta: Float) {
    let a = theta.sin();
    let b = theta.cos();
    let m: [Float; 9] = [1., 0., 0., 0., b, -a, 0., a, b];
    transform_vector2(vec, &m);
}

fn rotate_y(vec: &mut [Float; 3], theta: Float) {
    let a = theta.sin();
    let b = theta.cos();
    let m: [Float; 9] = [b, 0., a, 0., 1., 0., -a, 0., b];
    transform_vector2(vec, &m);
}

fn rotate_z(vec: &mut [Float; 3], theta: Float) {
    let a = theta.sin();
    let b = theta.cos();
    let m: [Float; 9] = [b, -a, 0., a, b, 0., 0., 0., 1.];
    transform_vector2(vec, &m);
}

fn clamp(mut x: Float, min: Float, max: Float) -> Float {
    if x < min {
        x = min;
    } else if x > max {
        x = max;
    }
    x
}
