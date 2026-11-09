//! Golden fixtures — six writer-level families against hand-maintained
//! raw-byte inputs (`cargo test` fully headless; design § 7; conventions
//! § Testing: raw byte fixtures, no ad-hoc ANSI built inside assertions).
//!
//! The seventh family of the spec's acceptance criteria — line-cap
//! eviction order — is a session-DOCUMENT policy and lives with
//! `SessionDocument` (`src/application/session.rs`, task T5).

use super::BlockTextWriter;
use crate::domain::output::{Color, Row};

/// Feed raw chunks through a fresh writer, finalize (EOF flush), and
/// collect the committed rows.
fn feed(cols: u16, chunks: &[&[u8]]) -> Vec<Row> {
    let mut writer = BlockTextWriter::new(cols);
    for chunk in chunks {
        writer.write(chunk);
    }
    writer.flush_active_row();
    writer.take_committed_rows()
}

fn texts(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| row.spans.iter().map(|span| span.text.as_str()).collect())
        .collect()
}

fn fg_of(row: &Row, span: usize) -> Color {
    row.spans[span].style.fg
}

fn bg_of(row: &Row, span: usize) -> Color {
    row.spans[span].style.bg
}

fn attrs_of(row: &Row, span: usize) -> (bool, bool, bool) {
    let style = &row.spans[span].style;
    (style.bold, style.italic, style.underline)
}

/// Family 1 — Unicode widths: CJK/emoji occupy two columns and drive wrap
/// points exactly like Latin text does.
#[test]
fn family_widths_cjk_emoji() {
    // '你' (2 cols): "ab你c" fills exactly 5 columns of a 5-col writer; X
    // starts the next row.
    let rows = feed(5, &[b"ab\xE4\xBD\xA0cX"]);
    assert_eq!(texts(&rows), ["ab你c", "X"]);

    // A wide glyph straddling the boundary commits the row first.
    let rows = feed(3, &[b"\xE4\xBD\xA0\xE5\xA5\xBD"]);
    assert_eq!(texts(&rows), ["你", "好"]);

    // Emoji are double-width too (and stay single spans in text stream).
    let rows = feed(4, &[b"\xF0\x9F\x98\x80\xE5\xA5\xBD"]);
    assert_eq!(texts(&rows), ["😀好"]);
}

/// Family 2 — SGR palette: 16-color, bright, 256-indexed, 24-bit RGB,
/// attribute set/reset — styles exactly as emitted, reset by SGR 0.
#[test]
fn family_sgr_palette() {
    let rows = feed(80, &[b"\x1b[31mred\x1b[1mbold\x1b[0mend"]);
    assert_eq!(texts(&rows), ["redboldend"]);
    assert_eq!(fg_of(&rows[0], 0), Color::Indexed(1));
    let (bold, _, _) = attrs_of(&rows[0], 1);
    assert!(bold);
    assert!(rows[0].spans[2].style.is_plain());

    let rows = feed(80, &[b"\x1b[38;5;196mA\x1b[38;2;1;2;3;48;5;42mB"]);
    assert_eq!(fg_of(&rows[0], 0), Color::Indexed(196));
    assert_eq!(fg_of(&rows[0], 1), Color::Rgb { r: 1, g: 2, b: 3 });
    assert_eq!(bg_of(&rows[0], 1), Color::Indexed(42));

    let rows = feed(80, &[b"\x1b[3;4mL\x1b[23ml_off"]);
    assert_eq!(attrs_of(&rows[0], 0), (false, true, true));
    assert_eq!(attrs_of(&rows[0], 1), (false, false, true));
}

/// Family 3 — wrap at the last column: deferred auto-wrap (VT100
/// pending-wrap), no fabricated empty rows from repeated last-column
/// writes.
#[test]
fn family_wrap_last_column() {
    let rows = feed(3, &[b"abcde"]);
    assert_eq!(texts(&rows), ["abc", "de"]);

    // Print at the last column twice: only the second printable char
    // commits — no blank row in between.
    let rows = feed(3, &[b"abX", b"Y\n"]);
    assert_eq!(texts(&rows), ["abX", "Y"]);
}

/// Family 4 — `\r` + `\x1b[K` rewrites: progress bars update in place,
/// `EL` paints with the active background at erase time.
#[test]
fn family_rewrite_progress_bar() {
    // Mid-flight chunked feed: each rewrote step is observable.
    let mut writer = BlockTextWriter::new(12);
    writer.write(b"\x1b[1m50%\x1b[0m");
    assert_eq!(texts(&writer.take_committed_rows()), Vec::<String>::new());

    writer.write(b"\r\x1b[2K\x1b[32m75%\x1b[0m\n");
    let rows = writer.take_committed_rows();
    assert_eq!(texts(&rows), ["75%"]);
    assert_eq!(fg_of(&rows[0], 0), Color::Indexed(2));

    // Rewrite at column 0 (LF kept column 3 after the 75% refresh):
    // 'done' lands at columns 0-3; BS drops back onto 'e' and EL 0 turns
    // it into one green-bg blank painted at erase time.
    writer.write(b"\rdone\x1b[42m\x08\x1b[K\n");
    let rows = writer.take_committed_rows();
    assert_eq!(texts(&rows), ["don "]);
    // blank painted with the active background (green).
    assert_eq!(bg_of(&rows[0], 1), Color::Indexed(2));
}

/// Family 5 — `EL`/`ED`: modes 0/1/2 over the current logical row; ED 3
/// (scrollback clear) ignored.
#[test]
fn family_el_ed_matrix() {
    // EL 0: erase from cursor to end.
    let rows = feed(16, &[b"progress 75%\r\x1b[K100%"]);
    let content = texts(&rows).join("");
    assert!(content.starts_with("100%"));
    assert_eq!(content.len(), 12); // erased tail = painted blanks

    // ED 2: whole current row cleared; the cursor does NOT move, so the
    // LF keeps its column and X lands at column 6 (staircase).
    let rows = feed(10, &[b"abcdef\x1b[2J\nX"]);
    assert_eq!(texts(&rows), ["      ", "      X"]);

    // ED 3: ignored (scrollback concept has no writer effect).
    let rows = feed(10, &[b"abc\x1b[3Jd"]);
    assert_eq!(texts(&rows), ["abcd"]);
}

/// Family 6 — UTF-8 multi-byte sequences split across chunk reads
/// reassemble correctly (mid-character cuts included).
#[test]
fn family_utf8_split_across_chunks() {
    // 'é' = C3 A9 ; '😀' = F0 9F 98 80 — cut mid-character.
    let rows = feed(20, &[b"h\xC3", b"\xA9\xF0\x9F", b"\x98\x80!"]);
    assert_eq!(texts(&rows), ["hé😀!"]);

    // '你' = E4 BD A0 — split inside the CJK glyph.
    let rows = feed(20, &[b"\xe4\xbd", b"\xa0\xe5\xa5\xbd\n"]);
    assert_eq!(texts(&rows), ["你好"]);
}
