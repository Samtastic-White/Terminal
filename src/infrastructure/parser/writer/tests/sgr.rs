//! SGR parsing: 16/256/24-bit colors, attributes, and malformed sequences.

use super::*;

#[test]
fn sgr_basic_16_foreground() {
    let rows = rows_of(80, &[b"\x1b[31mred"]);
    assert_eq!(rows[0].spans[0].style.fg, Color::Indexed(1));
    assert_eq!(rows[0].spans[0].text, "red");
}

#[test]
fn sgr_bright_and_aliases() {
    let rows = rows_of(80, &[b"\x1b[92mA\x1b[101mB"]);
    // SGR changes are incremental: the foreground set by 92 persists
    // while 101 switches only the background.
    assert_eq!(rows[0].spans[0].style.fg, Color::Indexed(10));
    assert_eq!(rows[0].spans[0].style.bg, Color::Default);
    assert_eq!(rows[0].spans[1].style.fg, Color::Indexed(10));
    assert_eq!(rows[0].spans[1].style.bg, Color::Indexed(9));
}

#[test]
fn sgr_256_palette() {
    let rows = rows_of(80, &[b"\x1b[38;5;196mfg\x1b[48;5;21mbg"]);
    // 48 replaces only the background; the foreground persists.
    assert_eq!(rows[0].spans[0].style.fg, Color::Indexed(196));
    assert_eq!(rows[0].spans[1].style.fg, Color::Indexed(196));
    assert_eq!(rows[0].spans[1].style.bg, Color::Indexed(21));
}

#[test]
fn sgr_truecolor() {
    let rows = rows_of(80, &[b"\x1b[38;2;255;10;20;48;2;1;2;3mX"]);
    assert_eq!(
        rows[0].spans[0].style.fg,
        Color::Rgb {
            r: 255,
            g: 10,
            b: 20
        }
    );
    assert_eq!(rows[0].spans[0].style.bg, Color::Rgb { r: 1, g: 2, b: 3 });
}

#[test]
fn sgr_attributes_and_full_reset() {
    let rows = rows_of(80, &[b"\x1b[1m\x1b[3m\x1b[4mb\x1b[0mplain"]);
    let first = &rows[0];
    assert!(first.spans[0].style.bold);
    assert!(first.spans[0].style.italic);
    assert!(first.spans[0].style.underline);
    assert!(first.spans[1].style.is_plain());
    assert_eq!(text(first), "bplain");
}

#[test]
fn sgr_malformed_sequences_degrade_without_panic() {
    let rows = rows_of(
        80,
        &[b"\x1b[38;5mX\x1b[38;2;1;2mY\x1b[999mZ\x1b]0;title\x07W"],
    );
    let single = rows.iter().map(text).collect::<Vec<_>>().join("");
    assert_eq!(single, "XYZW");
    assert!(rows[0].spans[0].style.is_plain());
}
