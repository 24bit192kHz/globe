//! Command line surface: the argument parser must accept every documented
//! value and reject everything else with a message a user can act on.
//!
//! Only non-rendering paths are exercised: the render modes need a tty.
use std::process::Command;

fn globe(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_globe"))
        .args(args)
        .output()
        .expect("run globe")
}

#[test]
fn help_lists_bodies_and_alphabets() {
    let out = globe(&["--help"]);
    assert!(out.status.success(), "globe --help failed");
    let text = String::from_utf8_lossy(&out.stdout);
    for body in globe::GlobeTemplate::NAMES {
        assert!(text.contains(body), "--help does not mention {}", body);
    }
    for glyph in ["ascii", "half", "braille"] {
        assert!(text.contains(glyph), "--help does not mention {}", glyph);
    }
    assert!(
        text.contains("recommended distance"),
        "--help lost the zoom default"
    );
}

#[test]
fn unknown_body_is_rejected_with_the_valid_values() {
    let out = globe(&["-t", "pluto", "-s"]);
    assert!(!out.status.success(), "pluto should not be a valid body");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("possible values") && text.contains("neptune"),
        "unhelpful rejection: {}",
        text
    );
}

#[test]
fn unknown_alphabet_is_rejected() {
    let out = globe(&["-G", "klingon", "-s"]);
    assert!(
        !out.status.success(),
        "klingon should not be a valid alphabet"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("possible values"),
        "unhelpful rejection"
    );
}

#[test]
fn unreadable_texture_exits_cleanly_before_touching_the_terminal() {
    let out = globe(&["-s", "--texture", "/nonexistent/globe-map.txt"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected a clean exit, not a panic"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("cannot read texture map") && !err.contains("panicked"),
        "unhelpful failure: {}",
        err
    );
}
