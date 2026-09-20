//! Render an ASCII globe in your terminal.

#![allow(unused_variables)]

use std::f32::consts::PI;
use std::fmt::Write as _;
use std::io::{stdin, stdout, Read, Stdout, Write};
use std::time::Duration;

use clap::{App, AppSettings, Arg};
use crossterm::{
    cursor,
    event::{poll, read, Event, KeyCode},
    ExecutableCommand,
};
use crossterm::{event::MouseEvent, terminal};

use crossterm::terminal::{ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use globe::{CameraConfig, Canvas, Globe, GlobeConfig, GlobeTemplate, Glyph};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

/// Collection of scene settings that get passed from clap to mode processing
/// functions.
struct Settings {
    /// Refresh rate in cycles per second
    refresh_rate: usize,
    /// Initial globe rotation speed
    globe_rotation_speed: f32,
    /// Initial camera rotation speed
    cam_rotation_speed: f32,
    /// Initial camera zoom, `None` for the body's recommended distance
    cam_zoom: Option<f32>,
    /// Target focus speed
    focus_speed: f32,
    /// Globe night side switch
    night: bool,
    /// Sub-cell glyph alphabet: render resolution per character cell
    glyph: Glyph,
    /// Built-in body to display
    template: GlobeTemplate,
    /// Custom day texture, overriding the template map
    texture: Option<String>,
    /// Custom night texture
    texture_night: Option<String>,
    /// Initial location coordinates
    coords: (f32, f32),
}

fn main() {
    let app = App::new("globe-cli")
        .version(VERSION)
        .author(AUTHORS)
        .setting(AppSettings::ArgRequiredElseHelp)
        .about("Render an ASCII globe in your terminal.")
        .arg(
            Arg::new("interactive")
                .short('i')
                .long("interactive")
                .display_order(0)
                .help("Interactive mode (input enabled)"),
        )
        .arg(
            Arg::new("screensaver")
                .short('s')
                .long("screensaver")
                .display_order(1)
                .help("Screensaver mode (input disabled)"),
        )
        .arg(
            Arg::new("refresh_rate")
                .short('r')
                .long("refresh-rate")
                .help("Refresh rate in frames per second")
                .takes_value(true)
                .value_name("fps")
                .default_value("30"),
        )
        .arg(
            Arg::new("globe_rotation")
                .short('g')
                .long("globe-rotation")
                .help("Starting globe rotation speed")
                .takes_value(true)
                .value_name("move_per_frame")
                .default_value("0"),
        )
        .arg(
            Arg::new("cam_rotation")
                .short('c')
                .long("cam-rotation")
                .help("Starting camera rotation speed")
                .takes_value(true)
                .value_name("move_per_frame")
                .default_value("0"),
        )
        .arg(
            Arg::new("cam_zoom")
                .short('z')
                .long("cam-zoom")
                .help("Starting camera zoom (defaults to the body's recommended distance)")
                .takes_value(true)
                .value_name("distance"),
        )
        .arg(
            Arg::new("focus_speed")
                .short('f')
                .long("focus-speed")
                .help("Target focusing animation speed")
                .takes_value(true)
                .value_name("multiplier")
                .default_value("1"),
        )
        .arg(
            Arg::new("location")
                .short('l')
                .long("location")
                .help("Starting location coordinates")
                .takes_value(true)
                .value_name("coords")
                .default_value("0.4,0.6"),
        )
        .arg(
            Arg::new("glyph")
                .short('G')
                .long("glyph")
                .help("Sub-cell glyph alphabet: ascii (1 sample/cell), half (2), braille (8)")
                .takes_value(true)
                .value_name("mode")
                .possible_values(["ascii", "half", "braille"])
                .default_value("braille"),
        )
        .arg(
            Arg::new("night")
                .short('n')
                .long("night")
                .help("Enable displaying the night side of the globe"),
        )
        .arg(
            Arg::new("template")
                .short('t')
                .long("template")
                .help("Built-in body to display")
                .takes_value(true)
                .value_name("planet")
                .possible_values(GlobeTemplate::NAMES)
                .default_value("earth"),
        )
        .arg(
            Arg::new("texture")
                .long("texture")
                .help("Apply custom texture from file (overrides the template day map)")
                .takes_value(true)
                .value_name("path"),
        )
        .arg(
            Arg::new("texture_night")
                .long("texture-night")
                .help("Apply custom night side texture from file")
                .takes_value(true)
                .value_name("path"),
        )
        .arg(
            Arg::new("pipe")
                .short('p')
                .long("pipe")
                .help("Read coordinates from stdin and display them on the globe"),
        );
    let matches = app.get_matches();

    // parse coordinates into a tuple
    let coords = matches
        .value_of("location")
        .unwrap()
        .split(",")
        .collect::<Vec<&str>>();
    if coords.len() != 2 {
        panic!("failed parsing location coordinates")
    }
    let coords: (f32, f32) = (
        coords[0]
            .parse()
            .expect("failed parsing location coordinates (first value)"),
        coords[1]
            .parse()
            .expect("failed parsing location coordinates (second value)"),
    );

    let settings = Settings {
        refresh_rate: matches
            .value_of("refresh_rate")
            .unwrap()
            .parse()
            .expect("failed parsing refresh rate value"),
        globe_rotation_speed: matches
            .value_of("globe_rotation")
            .unwrap()
            .parse()
            .expect("failed parsing globe rotation speed value"),
        cam_rotation_speed: matches
            .value_of("cam_rotation")
            .unwrap()
            .parse()
            .expect("failed parsing cam rotation speed value"),
        cam_zoom: matches
            .value_of("cam_zoom")
            .map(|v| v.parse().expect("failed parsing cam zoom value")),
        focus_speed: matches
            .value_of("focus_speed")
            .unwrap()
            .parse()
            .expect("failed parsing focus speed value"),
        night: matches.is_present("night"),
        glyph: Glyph::from_name(matches.value_of("glyph").unwrap()).expect("unknown glyph mode"),
        template: GlobeTemplate::from_name(matches.value_of("template").unwrap())
            .expect("unknown template"),
        texture: matches.value_of("texture").map(read_map),
        texture_night: matches.value_of("texture_night").map(read_map),
        coords,
    };

    if matches.is_present("pipe") {
        let stdin = stdin();
        let mut stdin_string = String::new();
        stdin.lock().read_to_string(&mut stdin_string).unwrap();
        let coord_list = stdin_string.split(";").collect::<Vec<&str>>();
        start_listing(settings, coord_list)
    } else if matches.is_present("interactive") {
        start_interactive(settings);
    } else if matches.is_present("screensaver") {
        start_screensaver(settings);
    }
}

/// Listing mode goes through a list of location coordinates. Pressing any key
/// triggers stepping to the next location, or if there are no more locations,
/// exits the program.
fn start_listing(settings: Settings, coords_input: Vec<&str>) {
    terminal::enable_raw_mode().unwrap();
    let mut stdout = stdout();
    stdout.execute(cursor::Hide).unwrap();
    stdout.execute(cursor::DisableBlinking).unwrap();

    let mut term_size = terminal::size().unwrap();
    let mut canvas = window_canvas(term_size);

    let mut cam_zoom = settings
        .cam_zoom
        .unwrap_or_else(|| settings.template.default_zoom());
    let mut cam_xy = 0.;
    let mut cam_z = 0.;

    let mut globe = build_globe(&settings, cam_zoom, cam_xy, cam_z);

    let coord_list: Vec<(f32, f32)> = coords_input
        .iter()
        .map(|c| {
            let split = c.split(",").collect::<Vec<&str>>();
            if split.len() != 2 {
                panic!("failed parsing coordinates, format: \"51.23,51.23\"");
            }
            (
                split[0]
                    .trim()
                    .parse()
                    .expect("failed parsing coord as float"),
                split[1]
                    .trim()
                    .parse()
                    .expect("failed parsing coord as float"),
            )
        })
        .collect();

    // set the initial coordinates
    focus_target(settings.coords, 0., &mut cam_xy, &mut cam_z);

    let globe_rot_speed = settings.globe_rotation_speed / 1000.;
    let cam_rot_speed = settings.cam_rotation_speed / 1000.;

    let mut current_index = 0;
    let mut moving_towards_target: Option<(f32, f32)> = Some(coord_list[current_index]);

    loop {
        if poll(Duration::from_millis(1000 / settings.refresh_rate as u64)).unwrap() {
            match read().unwrap() {
                // pressing any key steps to the next location, c and d quit
                Event::Key(key) => match key.code {
                    KeyCode::Char('c') | KeyCode::Char('d') => break,
                    _ => {
                        current_index += 1;
                        if current_index >= coord_list.len() {
                            break;
                        }
                        moving_towards_target = Some(coord_list[current_index]);
                    }
                },
                Event::Resize(width, height) => {
                    term_size = (width, height);
                    canvas = window_canvas(term_size);
                }
                Event::Mouse(_) => (),
            }
        }

        // apply globe rotation
        globe.angle += globe_rot_speed;
        cam_xy -= globe_rot_speed / 2.;

        // apply camera rotation
        cam_xy -= cam_rot_speed;

        if let Some(target_coords) = moving_towards_target {
            if move_towards_target(
                settings.focus_speed,
                target_coords,
                cam_zoom,
                globe.angle / 2.,
                &mut cam_xy,
                &mut cam_z,
                &mut cam_zoom,
            ) {
                moving_towards_target = None;
            }
        }

        globe.camera.update(cam_zoom, cam_xy, cam_z);

        // render globe on the canvas
        canvas.clear();
        globe.render_on(&mut canvas);

        // print canvas to terminal
        print_canvas(&canvas, &mut stdout);
    }

    stdout.execute(cursor::Show).unwrap();
    stdout.execute(cursor::EnableBlinking).unwrap();

    terminal::disable_raw_mode().unwrap();
    stdout.execute(terminal::Clear(ClearType::All)).unwrap();
}

/// Screensaver mode doesn't allow for user input. Any key press exits the
/// program.
fn start_screensaver(settings: Settings) {
    terminal::enable_raw_mode().unwrap();
    let mut stdout = stdout();
    stdout.execute(EnterAlternateScreen).unwrap();
    stdout.execute(cursor::Hide).unwrap();
    stdout.execute(cursor::DisableBlinking).unwrap();

    let mut term_size = terminal::size().unwrap();
    let mut canvas = fullscreen_canvas(term_size);
    // diff buffer: previous frame, sized in char cells
    let mut prev: Vec<char> = vec![' '; term_size.0 as usize * term_size.1 as usize];
    // one reused output buffer: changed cells are batched into runs
    let mut out = String::new();

    let cam_zoom = settings
        .cam_zoom
        .unwrap_or_else(|| settings.template.default_zoom());
    let mut cam_xy = 0.;
    let mut cam_z = 0.;

    // set the initial coordinates
    focus_target(settings.coords, 0., &mut cam_xy, &mut cam_z);

    let mut globe = build_globe(&settings, cam_zoom, cam_xy, cam_z);

    // equatorial orbit at ISS rate: camera circles equator,
    // one revolution per T = 92.9 min = 5574 s (ISS period, no inclination).
    // globe texture spins at true earth rate only (23.3 deg per orbit).
    const ISS_PERIOD_S: f32 = 5574.0;
    const EARTH_RATE: f32 = 2.0 * PI / 86164.0; // sidereal day rad/s
    let frame_dt = 1.0 / settings.refresh_rate as f32;
    let d_phase = 2.0 * PI / ISS_PERIOD_S * frame_dt;
    let d_earth = EARTH_RATE * frame_dt;
    let mut phase = 0.0f32;
    let lon0 = cam_xy;
    let lat0 = cam_z;
    let globe_rot_speed = settings.globe_rotation_speed / 1000.;
    let cam_rot_speed = settings.cam_rotation_speed / 1000.;
    let iss_mode = settings.globe_rotation_speed == 0.0 && settings.cam_rotation_speed == 0.0;
    // arrow-key spin boost: each press adds velocity, decays back to base.
    // right = spin up prograde, left = spin up retrograde.
    // up/down = tilt view latitude, clamped to camera limits.
    let mut boost_vel = 0.0f32;
    let mut lat_off = 0.0f32;
    const BOOST_STEP: f32 = 0.0025; // per keypress, rad/frame units of phase
    const BOOST_DECAY: f32 = 0.985; // per frame, ~2.3s to half
    const LAT_STEP: f32 = 0.05;

    loop {
        if poll(Duration::from_millis(1000 / settings.refresh_rate as u64)).unwrap() {
            match read().unwrap() {
                Event::Key(event) => match event.code {
                    KeyCode::Right => boost_vel += BOOST_STEP,
                    KeyCode::Left => boost_vel -= BOOST_STEP,
                    KeyCode::Up => lat_off = (lat_off + LAT_STEP).min(1.5),
                    KeyCode::Down => lat_off = (lat_off - LAT_STEP).max(-1.5),
                    _ => break,
                },
                Event::Resize(width, height) => {
                    term_size = (width, height);
                    canvas = fullscreen_canvas(term_size);
                    prev = vec![' '; width as usize * height as usize];
                    stdout.execute(terminal::Clear(ClearType::All)).unwrap();
                }
                Event::Mouse(_) => (),
            }
        }

        // decay boost toward zero, keep ISS base rate underneath
        boost_vel *= BOOST_DECAY;
        if boost_vel.abs() < 1e-7 {
            boost_vel = 0.0;
        }

        if iss_mode {
            // equatorial orbit at ISS rate: steady longitude sweep,
            // latitude fixed + user tilt. arrows add extra velocity on top.
            phase += d_phase + boost_vel;
            cam_xy = lon0 + phase;
            cam_z = (lat0 + lat_off).clamp(-1.5, 1.5);
            globe.angle += d_earth;
            cam_xy -= d_earth / 2.;
        } else {
            // manual spin mode: left/right push camera, up/down tilt
            globe.angle += globe_rot_speed;
            cam_xy -= globe_rot_speed / 2.;
            cam_xy -= cam_rot_speed + boost_vel;
            cam_z = (lat0 + lat_off).clamp(-1.5, 1.5);
        }

        globe.camera.update(cam_zoom, cam_xy, cam_z);

        // render globe on the canvas
        canvas.clear();
        globe.render_on(&mut canvas);

        // diffed print: only changed cells, batched, one flush per frame
        print_canvas_diff(&canvas, &mut prev, &term_size, &mut out, &mut stdout);
    }

    stdout.execute(cursor::Show).unwrap();
    stdout.execute(cursor::EnableBlinking).unwrap();
    stdout.execute(LeaveAlternateScreen).unwrap();

    terminal::disable_raw_mode().unwrap();
}

/// Interactive mode allows using mouse and/or keyboard to control the globe.
fn start_interactive(settings: Settings) {
    terminal::enable_raw_mode().unwrap();
    let mut stdout = stdout();
    stdout.execute(cursor::Hide).unwrap();
    stdout.execute(cursor::DisableBlinking).unwrap();
    stdout
        .execute(crossterm::event::EnableMouseCapture)
        .unwrap();

    let mut term_size = terminal::size().unwrap();
    let mut canvas = window_canvas(term_size);

    let mut cam_zoom = settings
        .cam_zoom
        .unwrap_or_else(|| settings.template.default_zoom());
    let mut cam_xy = 0.;
    let mut cam_z = 0.;

    // set the initial coordinates
    focus_target(settings.coords, 0., &mut cam_xy, &mut cam_z);

    let mut globe = build_globe(&settings, cam_zoom, cam_xy, cam_z);

    let mut globe_rot_speed = settings.globe_rotation_speed / 1000.;
    let mut cam_rot_speed = settings.cam_rotation_speed / 1000.;

    let mut last_drag_pos = None;
    let mut moving_towards_target: Option<(f32, f32)> = None;

    loop {
        if poll(Duration::from_millis(1000 / settings.refresh_rate as u64)).unwrap() {
            match read().unwrap() {
                Event::Key(event) => match event.code {
                    KeyCode::Char(char) => match char {
                        '-' => globe_rot_speed -= 0.005,
                        '+' => globe_rot_speed += 0.005,
                        ',' => cam_rot_speed -= 0.005,
                        '.' => cam_rot_speed += 0.005,
                        'n' => globe.display_night = !globe.display_night,
                        // vim-style navigation with hjkl
                        'h' => cam_xy += 0.1,
                        'l' => cam_xy -= 0.1,
                        'k' => {
                            if cam_z < 1.5 {
                                cam_z += 0.1;
                            }
                        }
                        'j' => {
                            if cam_z > -1.5 {
                                cam_z -= 0.1;
                            }
                        }
                        _ => break,
                    },
                    KeyCode::PageUp => cam_zoom += 0.1,
                    KeyCode::PageDown => cam_zoom -= 0.1,
                    KeyCode::Up => {
                        if cam_z < 1.5 {
                            cam_z += 0.1;
                        }
                    }
                    KeyCode::Down => {
                        if cam_z > -1.5 {
                            cam_z -= 0.1;
                        }
                    }
                    KeyCode::Left => cam_xy += 0.1,
                    KeyCode::Right => cam_xy -= 0.1,
                    KeyCode::Enter => {
                        focus_target(settings.coords, globe.angle / 2., &mut cam_xy, &mut cam_z);
                        // moving_towards_target = Some(settings.coords);
                    }
                    _ => (),
                },
                Event::Mouse(event) => match event {
                    MouseEvent::Drag(_, x, y, _) => {
                        if let Some(last) = last_drag_pos {
                            let (x_last, y_last) = last;
                            let x_diff = x as globe::Float - x_last as globe::Float;
                            let y_diff = y as globe::Float - y_last as globe::Float;

                            if y_diff > 0. && cam_z < 1.5 {
                                cam_z += 0.1;
                            } else if y_diff < 0. && cam_z > -1.5 {
                                cam_z -= 0.1;
                            }

                            cam_xy += x_diff * PI / 30.;
                            cam_xy += y_diff * PI / 30.;
                        }
                        last_drag_pos = Some((x, y))
                    }
                    MouseEvent::ScrollUp(..) => cam_zoom -= 0.1,
                    MouseEvent::ScrollDown(..) => cam_zoom += 0.1,
                    _ => last_drag_pos = None,
                },
                Event::Resize(width, height) => {
                    term_size = (width, height);
                    canvas = window_canvas(term_size);
                }
            }
        }

        // apply globe rotation
        globe.angle += globe_rot_speed;
        cam_xy -= globe_rot_speed / 2.;

        // apply camera rotation
        cam_xy -= cam_rot_speed;

        // clip camera zoom
        if cam_zoom < 1.0 {
            cam_zoom = 1.0;
        }

        if let Some(target_coords) = moving_towards_target {
            if move_towards_target(
                settings.focus_speed,
                target_coords,
                cam_zoom,
                globe.angle / 2.,
                &mut cam_xy,
                &mut cam_z,
                &mut cam_zoom,
            ) {
                moving_towards_target = None;
            }
        }

        globe.camera.update(cam_zoom, cam_xy, cam_z);

        // render globe on the canvas
        canvas.clear();
        globe.render_on(&mut canvas);

        // print canvas to terminal
        print_canvas(&canvas, &mut stdout);
    }

    stdout.execute(cursor::Show).unwrap();
    stdout.execute(cursor::EnableBlinking).unwrap();
    stdout
        .execute(crossterm::event::DisableMouseCapture)
        .unwrap();

    terminal::disable_raw_mode().unwrap();
    stdout.execute(terminal::Clear(ClearType::All)).unwrap();
}

/// Globe configured from the command line: the template supplies its baked
/// map, any explicit texture files replace it, then alphabet and camera.
fn build_globe(settings: &Settings, cam_zoom: f32, cam_xy: f32, cam_z: f32) -> Globe {
    let mut config = GlobeConfig::new()
        .use_template(settings.template)
        .with_glyph(settings.glyph)
        .with_camera(CameraConfig::new(cam_zoom, cam_xy, cam_z))
        .display_night(settings.night);
    if let Some(map) = &settings.texture {
        config = config.with_texture(map, None);
    }
    if let Some(map) = &settings.texture_night {
        config = config.with_night_texture(map, None);
    }
    config.build()
}

/// Reads an ascii texture map, failing with a message instead of a panic.
/// Called while parsing arguments, so a bad path never reaches the terminal.
fn read_map(path: &str) -> String {
    match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("globe: cannot read texture map {}: {}", path, err);
            std::process::exit(1);
        }
    }
}

/// Fullscreen canvas: one glyph per terminal cell, so the render grid is
/// exactly the terminal grid.
fn fullscreen_canvas(term_size: (u16, u16)) -> Canvas {
    Canvas::new(term_size.0, term_size.1, None)
}

/// Windowed canvas for the modes that draw in place: a globe disk that
/// fills the screen height, two cells wide per cell tall.
fn window_canvas(term_size: (u16, u16)) -> Canvas {
    if term_size.0 > term_size.1 {
        Canvas::new(term_size.1.saturating_mul(2), term_size.1, None)
    } else {
        Canvas::new(term_size.0, (term_size.0 / 2).max(1), None)
    }
}

/// Diffed fullscreen print: overwrite only changed cells, batched into runs
/// in one reused buffer with a single write and flush per frame, wrapped in
/// synchronized output so Kitty presents atomically, no tear. Unchanged
/// rows cost one slice compare, unchanged frames cost nothing at all.
fn print_canvas_diff(
    canvas: &Canvas,
    prev: &mut [char],
    term_size: &(u16, u16),
    out: &mut String,
    stdout: &mut Stdout,
) {
    let w = term_size.0 as usize;
    let h = term_size.1 as usize;
    out.clear();
    // synchronized output open
    out.push_str("\x1b[?2026h");
    let mut changed = 0usize;
    for y in 0..h {
        let row = &canvas.matrix[y * w..y * w + w];
        let prev_row = &mut prev[y * w..y * w + w];
        if row == prev_row {
            continue;
        }
        let mut x = 0;
        while x < w {
            if row[x] == prev_row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && row[x] != prev_row[x] {
                x += 1;
            }
            // cursor into place (ESC row;col H, the bytes MoveTo writes),
            // then the whole changed run as one string
            write!(out, "\x1b[{};{}H", y + 1, start + 1).unwrap();
            out.extend(row[start..x].iter());
            prev_row[start..x].copy_from_slice(&row[start..x]);
            changed += x - start;
        }
    }
    // synchronized output close
    out.push_str("\x1b[?2026l");
    if changed > 0 {
        stdout.write_all(out.as_bytes()).unwrap();
        stdout.flush().unwrap();
    }
}

/// Prints globe canvas to stdout, one cleared row at a time, and leaves the
/// cursor back where the canvas started so the next frame redraws in place.
/// Without that the windowed modes walk one row down the screen per frame and
/// smear the globe across the terminal.
fn print_canvas(canvas: &Canvas, stdout: &mut Stdout) {
    let (w, h) = canvas.get_size();
    let mut out = String::with_capacity(w * 4 + 24);
    for y in 0..h {
        out.clear();
        out.push_str("\x1b[2K"); // Clear(CurrentLine)
        out.extend(canvas.row(y).iter());
        // step down between rows only: stepping past the last row would
        // scroll the terminal once per frame
        if y + 1 < h {
            write!(out, "\x1b[1B\x1b[{}D", w).unwrap(); // MoveDown(1), MoveLeft
        } else {
            write!(out, "\x1b[{}D", w).unwrap(); // MoveLeft(w)
        }
        stdout.write_all(out.as_bytes()).unwrap();
        stdout.flush().unwrap();
    }
    write!(out, "\x1b[{}A", h.saturating_sub(1)).unwrap(); // back to the top row
    stdout.write_all(out.as_bytes()).unwrap();
    stdout.flush().unwrap();
}

/// Orients the camera so that it focuses on the given target coordinates.
pub fn focus_target(coords: (f32, f32), xy_offset: f32, cam_xy: &mut f32, cam_z: &mut f32) {
    let (cx, cy) = coords;
    *cam_xy = -(cx * PI) - 1.5 - xy_offset;
    *cam_z = cy * 3. - 1.5;
}

//TODO animate zoom
/// Rotates the camera towards given target coordinates.
pub fn move_towards_target(
    speed: f32,
    coords: (f32, f32),
    target_zoom: f32,
    xy_offset: f32,
    cam_xy: &mut f32,
    cam_z: &mut f32,
    cam_zoom: &mut f32,
) -> bool {
    let (cx, cy) = coords;
    let target_xy = -(cx * PI - xy_offset) - 1.5;
    let target_z = cy * 3. - 1.5;

    let diff_xy = target_xy - *cam_xy;
    let diff_z = target_z - *cam_z;

    if diff_xy.abs() < 0.01 && diff_z.abs() < 0.01 {
        return true;
    }

    let mut xy_move = 0.01 * speed + (diff_xy.abs() / 30. * speed);
    if diff_xy.abs() < 0.07 {
        xy_move /= 5.;
    }
    if diff_xy > 0. {
        *cam_xy += xy_move;
    } else if diff_xy < 0. {
        *cam_xy -= xy_move;
    }

    let mut z_move = 0.005 * speed + (diff_z.abs() / 30. * speed);
    if diff_z.abs() < 0.07 {
        z_move /= 5.;
    }
    if diff_z > 0. {
        *cam_z += z_move;
    } else if diff_z < 0. {
        *cam_z -= z_move;
    }

    false
}
