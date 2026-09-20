//! Texture pipeline: the shipped maps, the day/night pairing and the ascii
//! loader's tolerance for real world input.
use globe::{Canvas, GlobeConfig, GlobeTemplate, Glyph, Texture};

const PALETTE: &str = " .:;',wiogOLXHWYV@";
const MAP_SIZE: (usize, usize) = (1024, 512);

/// Every body ships a baked map of the same size and palette, and it parses:
/// a corrupt or missing map would panic here instead of at the first frame.
#[test]
fn every_body_ships_a_map() {
    for body in GlobeTemplate::ALL {
        let globe = GlobeConfig::new().use_template(body).build();
        assert_eq!(globe.texture.size(), MAP_SIZE, "{} map size", body.name());
        assert_eq!(
            globe.texture.palette().iter().collect::<String>(),
            PALETTE,
            "{} palette",
            body.name()
        );
    }
}

/// Only earth has a night side, so `-n` must change earth's render and leave
/// the other bodies character-identical.
#[test]
fn only_earth_reacts_to_the_night_switch() {
    let render = |body, night| {
        let mut canvas = Canvas::new(60, 30, None);
        let mut globe = GlobeConfig::new()
            .use_template(body)
            .with_glyph(Glyph::Ascii)
            .display_night(night)
            .build();
        canvas.clear();
        globe.render_on(&mut canvas);
        canvas.matrix
    };
    assert_ne!(
        render(GlobeTemplate::Earth, false),
        render(GlobeTemplate::Earth, true),
        "earth night side changed nothing"
    );
    for body in GlobeTemplate::ALL
        .iter()
        .copied()
        .filter(|b| *b != GlobeTemplate::Earth)
    {
        assert_eq!(
            render(body, false),
            render(body, true),
            "{} has no night map, so -n must be a no-op",
            body.name()
        );
    }
}

/// Ragged maps are padded and unknown glyphs extend the palette, so a hand
/// written texture renders as written instead of panicking. Rows are stored
/// mirrored (the loader reverses each line, as the text format always has).
#[test]
fn ascii_maps_tolerate_real_world_input() {
    let texture = Texture::from_ascii("ab\nabc", None);
    assert_eq!(texture.size(), (3, 2), "ragged rows padded to the widest");
    let palette: String = texture.palette().iter().collect();
    assert_eq!(palette, "bac", "glyphs are appended in first-seen order");
}

/// A night map that does not match the day map's size is a configuration
/// error, and it must be caught rather than read out of bounds later.
#[test]
#[should_panic(expected = "night texture must match the day texture size")]
fn mismatched_night_map_is_rejected() {
    let mut texture = Texture::from_ascii("..\n..", None);
    texture.set_night_ascii("...\n...\n...");
}
