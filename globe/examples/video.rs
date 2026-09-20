//! Throwaway: render a deterministic 1080p frame sequence to stdout as raw
//! 8-bit gray frames, for piping into ffmpeg.
//!
//! usage: video BODY FRAMES ZOOM [night] [orbit_turns] [dot] [spin]
//!
//! `dot` is how many output pixels one braille dot covers: 1 renders a
//! 960x270 cell grid, which is exactly 1920x1080 dots (one dot per pixel),
//! 2 halves the cell grid and repeats every dot over 2x2 pixels. The camera
//! walks `orbit_turns` revolutions over the clip and the globe spins by
//! `spin` radians per frame, so the output is smooth at any frame rate
//! regardless of how long each frame takes to render.
use std::io::Write;

use globe::{CameraConfig, Canvas, GlobeConfig, GlobeTemplate, Glyph};

/// Braille dot bits, `[column][row]`, matching the renderer's own table.
const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let body = GlobeTemplate::from_name(&args[1]).expect("unknown body");
    let frames: usize = args[2].parse().expect("frames");
    let zoom: f32 = args[3].parse().expect("zoom");
    let night = args.iter().any(|a| a == "night");
    let turns: f32 = args.get(4).and_then(|t| t.parse().ok()).unwrap_or(1.0);
    let dot: usize = args.get(5).and_then(|d| d.parse().ok()).unwrap_or(1);
    let spin: f32 = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(0.0015);

    let (cols, rows) = (960 / dot, 270 / dot);
    let mut canvas = Canvas::new(cols as u16, rows as u16, None);
    let mut globe = GlobeConfig::new()
        .use_template(body)
        .with_glyph(Glyph::Braille)
        .with_camera(CameraConfig::new(zoom, 0., 0.3))
        .display_night(night)
        .build();

    // start where the screensaver's default target aims
    let lon0 = -0.4 * std::f32::consts::PI - 1.5;
    let lat = 0.3f32;
    let mut frame = Vec::with_capacity(1920 * 1080);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for f in 0..frames {
        let turn = f as f32 / frames as f32 * turns * 2. * std::f32::consts::PI;
        globe.camera.update(zoom, lon0 + turn, lat);
        globe.angle += spin;
        canvas.clear();
        globe.render_on(&mut canvas);

        frame.clear();
        for y in 0..rows {
            let row = canvas.row(y);
            for dot_row in 0..4 {
                for _ in 0..dot {
                    for cell in row.iter() {
                        let mask = (*cell as u32).wrapping_sub(0x2800);
                        // BITS is indexed [column][row]: walk the columns
                        for column in BITS.iter() {
                            let value = if mask & column[dot_row] != 0 {
                                255u8
                            } else {
                                0u8
                            };
                            for _ in 0..dot {
                                frame.push(value);
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(frame.len(), 1920 * 1080);
        out.write_all(&frame).unwrap();
        if f % 20 == 0 {
            eprintln!("frame {}/{}", f, frames);
        }
    }
}
