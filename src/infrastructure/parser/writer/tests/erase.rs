//! `EL`/`ED` erase semantics.

use super::*;

#[test]
fn el_zero_erases_to_end_of_row() {
    // Cursor moves within the subset only (BS): from column 6 back to
    // 4; the erased range becomes painted blanks (default bg here).
    let rows = rows_of(10, &[b"abcdef\x08\x08\x1b[K"]);
    assert_eq!(text(&rows[0]), "abcd  ");
}

#[test]
fn el_one_erases_through_cursor_inclusively() {
    let rows = rows_of(10, &[b"abcdef\x08\x08\x08\x1b[1K"]);
    assert_eq!(text(&rows[0]), "    ef");
}

#[test]
fn ed_two_clears_current_row_only() {
    let rows = rows_of(10, &[b"abcdef\x1b[2J\nX"]);
    // ED 2 blanks the current logical row; the cursor does NOT move,
    // so the LF keeps column 6 and X lands there (staircase).
    assert_eq!(
        rows.iter().map(text).collect::<Vec<_>>(),
        ["      ", "      X"]
    );
}

#[test]
fn ed_three_scrollback_clear_is_ignored() {
    let rows = rows_of(10, &[b"abc\x1b[3Jd"]);
    assert_eq!(text(&rows[0]), "abcd");
}

#[test]
fn erase_paints_with_active_background() {
    // EL 0 from column 0: both cells become blanks carrying the bg
    // active at erase time (green here).
    let rows = rows_of(10, &[b"\x1b[42mAB\x08\x08\x1b[K"]);
    for span in &rows[0].spans {
        assert_eq!(span.style.bg, Color::Indexed(2));
    }
    assert_eq!(text(&rows[0]), "  ");
}
