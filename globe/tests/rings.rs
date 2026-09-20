//! Rings: the geometry the renderer relies on, and the fact that a ringed
//! body actually draws matter outside its own silhouette.

use globe::{CameraConfig, Canvas, GlobeConfig, GlobeTemplate, Glyph, Ring};

/// Ring profiles are embedded per template; only Saturn has one.
#[test]
fn only_saturn_ships_a_ring() {
    for body in GlobeTemplate::ALL {
        let globe = GlobeConfig::new().use_template(body).build();
        assert_eq!(
            globe.ring.is_some(),
            body == GlobeTemplate::Saturn,
            "{} rings",
            body.name()
        );
    }
}

/// The baked profile must describe a sane annulus: inner edge outside the
/// planet, outer edge beyond it, and a tilt within a plausible obliquity.
#[test]
fn saturn_ring_geometry_is_sane() {
    let ring = GlobeConfig::new()
        .use_template(GlobeTemplate::Saturn)
        .build()
        .ring
        .expect("saturn has rings");
    assert!(
        ring.inner() > 1.0,
        "inner edge {} should clear the planet radius of 1",
        ring.inner()
    );
    assert!(
        ring.outer() > ring.inner() + 0.5,
        "ring spans {}..{}",
        ring.inner(),
        ring.outer()
    );
    assert!(ring.tilt().abs() <= std::f32::consts::PI / 2.0);
    assert!(ring.count() > 64, "profile has {} samples", ring.count());
}

/// Lit cells in the outermost tenths of a wide canvas. The globe's disk is
/// much narrower than that there, so anything lit out there is star field or
/// ring matter: a ringless body and a ringed one differ by exactly the rings.
fn outer_lit_cells(body: GlobeTemplate) -> usize {
    let (dw, dh) = (200usize, 40usize);
    let mut canvas = Canvas::new(dw as u16, dh as u16, None);
    // the body's recommended distance, which is where rings must read wide
    let camera = CameraConfig::new(body.default_zoom(), 0.0, 0.3);
    let mut globe = GlobeConfig::new()
        .use_template(body)
        .with_camera(camera)
        .with_glyph(Glyph::Ascii)
        .build();
    canvas.clear();
    globe.render_on(&mut canvas);
    let margin = dw / 10;
    (0..dh)
        .flat_map(|y| {
            canvas.row(y)[..margin]
                .iter()
                .chain(&canvas.row(y)[dw - margin..])
        })
        .filter(|c| **c != ' ')
        .count()
}

/// From inside the ring band the near half of the rings sits behind the
/// camera, so the recommended view must sit outside it.
#[test]
fn saturn_is_viewed_from_outside_its_rings() {
    let ring = GlobeConfig::new()
        .use_template(GlobeTemplate::Saturn)
        .build()
        .ring
        .expect("saturn has rings");
    assert!(
        GlobeTemplate::Saturn.default_zoom() > ring.outer() + 1.0,
        "zoom {} vs ring outer {}",
        GlobeTemplate::Saturn.default_zoom(),
        ring.outer()
    );
}

/// A ring lights cells the globe cannot reach, so Saturn must draw matter in
/// the far columns that a ringless body leaves to the star field.
#[test]
fn rings_light_cells_beyond_the_globe() {
    let jupiter = outer_lit_cells(GlobeTemplate::Jupiter);
    let saturn = outer_lit_cells(GlobeTemplate::Saturn);
    assert!(
        saturn > jupiter,
        "saturn lit {} outer cells, ringless jupiter {}",
        saturn,
        jupiter
    );
}

/// A truncated or foreign profile must be rejected, not misread.
#[test]
#[should_panic(expected = "RING1")]
fn truncated_ring_profile_is_rejected() {
    static NOT_A_PROFILE: [u8; 8] = [0; 8];
    Ring::parse(&NOT_A_PROFILE);
}
