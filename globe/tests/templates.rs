//! End-to-end guard for the body wiring: every [`GlobeTemplate`] must
//! resolve by name and render a distinct, non-trivial globe.
use globe::{Canvas, GlobeConfig, GlobeTemplate, Glyph};

#[test]
fn template_names_round_trip() {
    for t in GlobeTemplate::ALL {
        assert_eq!(
            GlobeTemplate::from_name(t.name()),
            Some(t),
            "from_name({:?}) did not round-trip",
            t.name()
        );
    }
}

#[test]
fn template_names_match_all() {
    assert_eq!(
        GlobeTemplate::NAMES.len(),
        GlobeTemplate::ALL.len(),
        "NAMES length does not match ALL length"
    );
    for (i, t) in GlobeTemplate::ALL.iter().enumerate() {
        assert_eq!(
            GlobeTemplate::NAMES[i],
            t.name(),
            "NAMES[{}] does not match ALL[{}]",
            i,
            i
        );
    }
}

#[test]
fn all_templates_render_distinct() {
    const COLS: usize = 80;
    const ROWS: usize = 40;
    const CELLS: usize = COLS * ROWS;

    let mut renders: Vec<Vec<char>> = Vec::with_capacity(GlobeTemplate::ALL.len());
    for t in GlobeTemplate::ALL {
        let mut globe = GlobeConfig::new()
            .use_template(t)
            .with_glyph(Glyph::Braille)
            .build();
        let mut canvas = Canvas::new(COLS as u16, ROWS as u16, None);
        globe.render_on(&mut canvas);
        // a blank braille cell is U+2800, not a space, so count lit dots
        let lit = canvas
            .matrix
            .iter()
            .filter(|c| **c != ' ' && **c != '\u{2800}')
            .count();
        // bodies sit between 33% and 48% lit at this size; 25% catches a
        // template whose map failed to load without pinning the exact look
        assert!(
            lit * 4 >= CELLS,
            "{:?}: only {}/{} cells have lit dots",
            t,
            lit,
            CELLS
        );
        renders.push(canvas.matrix);
    }

    let mut distinct = 0;
    let mut total = 0;
    for i in 0..renders.len() {
        for j in (i + 1)..renders.len() {
            total += 1;
            if renders[i] != renders[j] {
                distinct += 1;
            }
        }
    }
    assert!(
        distinct == total,
        "only {}/{} renders are pairwise distinct",
        distinct,
        total
    );
}
