//! `Glyph::{name, from_name, sub}` consistency for all three alphabets.
use globe::Glyph;

const ALL: [(Glyph, &str, (usize, usize), usize); 3] = [
    (Glyph::Ascii, "ascii", (1, 1), 1),
    (Glyph::Half, "half", (1, 2), 2),
    (Glyph::Braille, "braille", (2, 4), 8),
];

#[test]
fn glyph_names_round_trip() {
    for (glyph, name, _, _) in ALL {
        assert_eq!(glyph.name(), name, "{glyph:?}.name()");
        assert_eq!(
            Glyph::from_name(name),
            Some(glyph),
            "from_name({name:?}) did not round-trip"
        );
    }
}

#[test]
fn glyph_sub_matches_documented_sample_counts() {
    for (glyph, name, (cols, rows), samples) in ALL {
        assert_eq!(glyph.sub(), (cols, rows), "{name} sub-cell grid changed");
        assert_eq!(
            cols * rows,
            samples,
            "{name} carries {} samples/cell, expected {samples}",
            cols * rows
        );
    }
}

#[test]
fn glyph_from_name_rejects_unknown() {
    assert_eq!(Glyph::from_name("bogus"), None);
    assert_eq!(Glyph::from_name("ASCII"), None);
}
