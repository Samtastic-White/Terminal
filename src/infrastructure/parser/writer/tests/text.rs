//! Text printing, C0 controls, auto-wrap, and active-row behavior.

use super::*;

#[test]
fn plain_text_single_row() {
    let rows = rows_of(80, &[b"hello"]);
    assert_eq!(rows.len(), 1);
    assert_eq!(text(&rows[0]), "hello");
    assert!(rows[0].spans[0].style.is_plain());
    assert!(rows[0].spans[0].text == "hello");
}

#[test]
fn crlf_breaks_rows() {
    let rows = rows_of(80, &[b"a\r\nb"]);
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a", "b"]);
}

#[test]
fn bare_lf_keeps_column_and_blank_row_is_content() {
    let rows = rows_of(10, &[b"a\n\nb"]);
    // Double LF is real blank-line content between rows; the LF keeps
    // the column, so 'b' lands at column 1 (staircase by design).
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a", "", " b"]);
}

#[test]
fn bs_clamps_at_floor_zero() {
    let rows = rows_of(10, &[b"\x08a\x08"]);
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a"]);
}

#[test]
fn ht_snaps_to_tab_stop_and_keeps_position() {
    let rows = rows_of(16, &[b"\tab"]);
    // Tab stops at column 8: the unwritten prefix is rendered as the
    // blank cells it occupies (leading padding).
    assert_eq!(text(&rows[0]), "        ab");
}

#[test]
fn ht_clamps_at_last_column_without_fabricating_rows() {
    let rows = rows_of(5, &[b"\t\t\t\tX"]);
    assert_eq!(rows.len(), 1);
    assert_eq!(text(&rows[0]), "    X");
}

#[test]
fn deferred_wrap_at_last_column_commits_once() {
    let rows = rows_of(5, &[b"abcde", b"x"]);
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["abcde", "x"]);
}

#[test]
fn trailing_lf_never_fabricates_rows() {
    let rows = rows_of(10, &[b"a\n"]);
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a"]);
}

#[test]
fn active_row_exposes_prompt_and_typing_without_committing() {
    let mut writer = BlockTextWriter::new(80);
    writer.write(b"\x1b[32muser@host:~$ \x1b[0m");

    let prompt = writer.active_row().expect("prompt must be live");
    assert_eq!(text(&prompt), "user@host:~$ ");
    assert_eq!(prompt.spans[0].style.fg, Color::Indexed(2));
    assert!(writer.take_committed_rows().is_empty());

    writer.write(b"ls");
    assert_eq!(text(&writer.active_row().unwrap()), "user@host:~$ ls");
    assert!(writer.take_committed_rows().is_empty());
}

#[test]
fn active_row_rewrites_in_place_and_retires_on_commit() {
    let mut writer = BlockTextWriter::new(80);
    writer.write(b"50%");
    writer.write(b"\r75%");
    assert_eq!(text(&writer.active_row().unwrap()), "75%");
    assert!(writer.take_committed_rows().is_empty());

    writer.write(b"\r\n");
    assert!(writer.active_row().is_none());
    let rows = writer.take_committed_rows();
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["75%"]);

    // Flush is the other retirement path (PTY EOF).
    writer.write(b"tail");
    assert_eq!(text(&writer.active_row().unwrap()), "tail");
    writer.flush_active_row();
    assert!(writer.active_row().is_none());
    let rows = writer.take_committed_rows();
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["tail"]);
}

#[test]
fn cr_rewrites_overwrite_in_place() {
    let rows = rows_of(10, &[b"hello\rX\n"]);
    // \r moves the cursor but does not erase: X overwrites 'h'.
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["Xello"]);
}

#[test]
fn resize_updates_wrap_width_for_new_bytes() {
    let mut writer = BlockTextWriter::new(10);
    writer.write(b"verylongline\r\n");
    // 12 columns of text wrap at width 10 into two committed rows that
    // keep their creation-time width.
    let rows = writer.take_committed_rows();
    assert_eq!(
        rows.iter().map(text).collect::<Vec<_>>(),
        ["verylongli", "ne"]
    );

    writer.resize_cols(40);
    assert_eq!(writer.cols(), 40);
    writer.write(b"short\n");
    let rows = writer.take_committed_rows();
    assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["short"]);
}
