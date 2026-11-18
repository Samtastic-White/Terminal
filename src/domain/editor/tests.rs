//! Editor and display-map unit tests.

use super::*;

fn editor(text: &str) -> Editor {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(80);
    editor.set_text(text);
    editor
}

fn editor_at(text: &str, line: usize, cell: usize) -> Editor {
    let mut editor = editor(text);
    editor.move_to_cell(line, cell);
    editor
}

fn limited(max_bytes: usize, max_lines: usize) -> EditorLimits {
    EditorLimits {
        max_bytes,
        max_lines,
    }
}

/// Builds a selection directly; tests own the invariants by construction.
fn select(editor: &mut Editor, start: usize, end: usize) {
    editor.caret = start;
    editor.anchor = Some(end);
}

fn repeated_lines(count: usize, width: usize) -> String {
    let line = "a".repeat(width);
    (0..count)
        .map(|_| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_new_editor_is_empty_and_unselected() {
    let editor = Editor::default();

    assert_eq!(editor.text(), "");
    assert_eq!(editor.caret(), 0);
    assert!(!editor.has_selection());
    assert_eq!(editor.visual_line_count(), 1);
    assert_eq!(editor.caret_visual_position(), (0, 0));
    assert_eq!(editor.revision(), 0);
}

#[test]
fn typing_inserts_at_the_caret_and_advances_it() {
    let mut editor = editor("");

    assert!(editor.insert_text("ls -la").inserted);

    assert_eq!(editor.text(), "ls -la");
    assert_eq!(editor.caret(), 6);
    assert_eq!(editor.revision(), 1);
}

#[test]
fn typing_in_the_middle_splits_the_text() {
    let mut editor = editor_at("abc", 0, 1);

    assert!(editor.insert_text("XY").inserted);

    assert_eq!(editor.text(), "aXYbc");
    assert_eq!(editor.caret(), 3);
}

#[test]
fn typing_replaces_the_selection() {
    let mut editor = editor("hello world");
    select(&mut editor, 6, 11);
    assert!(editor.has_selection());

    assert!(editor.insert_text("there").inserted);

    assert_eq!(editor.text(), "hello there");
    assert!(!editor.has_selection());
    assert_eq!(editor.caret(), 11);
}

#[test]
fn typed_text_is_sanitized_and_empty_input_is_ignored() {
    let mut editor = editor("a");

    assert!(!editor.insert_text("").inserted);
    assert!(!editor.insert_text("\u{1b}").inserted);
    assert_eq!(editor.text(), "a");

    assert!(editor.insert_text("\tb").inserted);
    assert_eq!(editor.text(), "a\tb");
}

#[test]
fn typing_at_the_byte_cap_is_ignored_atomically() {
    let mut editor = Editor::new(limited(4, 512));
    editor.set_wrap_cols(20);
    editor.set_text("abcd");
    assert_eq!(editor.caret(), 4);

    assert!(!editor.insert_text("e").inserted);

    assert_eq!(editor.text(), "abcd");
    assert_eq!(editor.caret(), 4);
}

#[test]
fn typing_a_line_break_at_the_line_cap_is_ignored() {
    let mut editor = Editor::new(limited(100, 2));
    editor.set_wrap_cols(20);
    editor.set_text("a\nb");

    assert!(!editor.insert_text("\n").inserted);
    assert_eq!(editor.text(), "a\nb");

    assert!(editor.insert_text("c").inserted);
    assert_eq!(editor.text(), "a\nbc");
}

#[test]
fn replacing_a_selection_frees_budget_for_new_text() {
    let mut editor = Editor::new(limited(4, 512));
    editor.set_wrap_cols(20);
    editor.set_text("abcd");
    assert!(editor.select_all());

    assert!(editor.insert_text("wxyz").inserted);

    assert_eq!(editor.text(), "wxyz");
    assert_eq!(editor.caret(), 4);
}

#[test]
fn deleting_backward_joins_lines() {
    let mut editor = editor_at("ab\ncd", 1, 0);

    assert!(editor.delete_backward());

    assert_eq!(editor.text(), "abcd");
    assert_eq!(editor.caret(), 2);
}

#[test]
fn deleting_forward_joins_lines() {
    let mut editor = editor_at("ab\ncd", 0, 2);

    assert!(editor.delete_forward());

    assert_eq!(editor.text(), "abcd");
    assert_eq!(editor.caret(), 2);
}

#[test]
fn deletes_at_the_text_edges_are_no_ops() {
    let mut editor = editor_at("ab", 0, 0);
    assert!(!editor.delete_backward());

    editor.move_end(false);
    assert!(!editor.delete_forward());

    assert_eq!(editor.text(), "ab");
}

#[test]
fn deleting_multibyte_characters_keeps_char_boundaries() {
    let mut editor = editor("aé漢");

    assert!(editor.delete_backward());
    assert_eq!(editor.text(), "aé");
    assert_eq!(editor.caret(), 3);

    assert!(editor.delete_backward());
    assert_eq!(editor.text(), "a");
    assert_eq!(editor.caret(), 1);
    assert!(editor.text().is_char_boundary(editor.caret()));
}

#[test]
fn deleting_a_selection_removes_it_whole() {
    let mut editor = editor("keep DELETE");
    select(&mut editor, 5, 11);

    assert!(editor.delete_backward());

    assert_eq!(editor.text(), "keep ");
}

#[test]
fn select_all_covers_the_whole_buffer() {
    let mut editor = editor("first\nsecond");

    assert!(editor.select_all());

    assert_eq!(editor.selection_range(), Some(0..12));
    assert_eq!(editor.selected_text(), Some("first\nsecond"));
    assert_eq!(editor.caret(), 12);
}

#[test]
fn a_collapsed_anchor_is_not_a_selection() {
    let mut editor = editor("abc");
    editor.move_to_cell(0, 1);
    assert!(editor.move_horizontal(1, true));
    assert!(editor.has_selection());
    assert!(editor.move_horizontal(-1, true));

    assert!(!editor.has_selection());
    assert_eq!(editor.selection_range(), None);
    assert_eq!(editor.selected_text(), None);
}

#[test]
fn a_plain_horizontal_move_collapses_the_selection_to_its_edge() {
    let mut editor = editor("abcdef");
    select(&mut editor, 1, 6);

    assert!(editor.move_horizontal(-1, false));
    assert_eq!(editor.caret(), 1);

    select(&mut editor, 1, 6);
    assert!(editor.move_horizontal(1, false));
    assert_eq!(editor.caret(), 6);
}

#[test]
fn vertical_motion_preserves_the_desired_column() {
    let mut editor = editor("abcdef\nxy\nabcdef");
    editor.move_to_cell(0, 5);

    assert!(editor.move_vertical(Direction::Down, false));
    assert_eq!(editor.caret_visual_position(), (1, 2));

    assert!(editor.move_vertical(Direction::Down, false));
    assert_eq!(editor.caret_visual_position(), (2, 5));
}

#[test]
fn horizontal_motion_clears_the_desired_column() {
    let mut editor = editor("abcdef\nxy\nabcdef");
    editor.move_to_cell(0, 5);
    editor.move_vertical(Direction::Down, false);

    editor.move_horizontal(-1, false);
    editor.move_vertical(Direction::Down, false);

    assert_eq!(editor.caret_visual_position(), (2, 1));
}

#[test]
fn vertical_motion_stops_at_the_first_and_last_visual_line() {
    let mut editor = editor("one\ntwo");
    editor.move_to_cell(0, 0);

    assert!(!editor.move_vertical(Direction::Up, false));
    editor.move_vertical(Direction::Down, false);
    assert!(!editor.move_vertical(Direction::Down, false));
}

#[test]
fn home_and_end_follow_logical_lines_not_visual_ones() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(10);
    editor.set_text("abcdefghijkl\nxy");
    editor.move_to_cell(0, 7);
    assert_eq!(editor.visual_line_count(), 3);

    assert!(editor.move_home(false));
    assert_eq!(editor.caret(), 0);

    assert!(editor.move_end(false));
    assert_eq!(editor.caret(), 12);
}

#[test]
fn page_motion_moves_by_the_visible_row_count() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(80);
    editor.set_visible_lines(2);
    editor.set_text("1\n2\n3\n4\n5");
    editor.move_to_cell(0, 0);

    assert!(editor.move_page(Direction::Down, false));
    assert_eq!(editor.caret_visual_position(), (2, 0));

    assert!(editor.move_page(Direction::Up, false));
    assert_eq!(editor.caret_visual_position(), (0, 0));

    assert!(!editor.move_page(Direction::Up, false));
}

#[test]
fn clicks_round_to_the_nearest_character_boundary() {
    let mut editor = editor("漢字");

    assert!(editor.move_to_cell(0, 1));
    assert_eq!(editor.caret(), 3);

    assert!(editor.move_to_cell(0, 0));
    assert_eq!(editor.caret(), 0);

    assert!(editor.move_to_cell(0, 999));
    assert_eq!(editor.caret(), 6);
}

#[test]
fn dragging_extends_from_the_click_without_losing_the_anchor() {
    let mut editor = editor_at("hello", 0, 1);

    assert!(editor.drag_to_cell(0, 4));
    assert_eq!(editor.selected_text(), Some("ell"));
    assert_eq!(editor.caret(), 4);

    assert!(editor.drag_to_cell(0, 2));
    assert_eq!(editor.selected_text(), Some("e"));

    editor.move_to_cell(0, 3);
    assert!(!editor.has_selection());
}

#[test]
fn clicks_inside_a_tab_land_on_the_tab_byte() {
    let mut editor = editor("a\tb");

    assert!(editor.move_to_cell(0, 2));

    assert_eq!(editor.caret(), 1);
}

#[test]
fn clicks_below_the_last_line_land_at_the_end() {
    let mut editor = editor_at("one\ntwo", 0, 0);

    assert!(editor.move_to_cell(99, 0));

    assert_eq!(editor.caret(), 7);
}

#[test]
fn the_display_map_expands_tabs_to_four_cell_stops() {
    let map = DisplayMap::new("a\tb", 80);

    assert_eq!(map.lines().len(), 1);
    assert_eq!(map.lines()[0].cells, 5);
    assert_eq!(map.byte_at(0, 2), 1);
    assert_eq!(map.cell_of(2), 4);
}

#[test]
fn the_display_map_measures_wide_and_zero_width_characters() {
    let map = DisplayMap::new("e\u{301}x漢", 80);
    let line = &map.lines()[0];

    assert_eq!(line.cells, 4);
    assert_eq!(map.cell_of(3), 1);
    assert_eq!(map.cell_of(4), 2);
    assert_eq!(map.byte_at(0, 1), 3);
    assert_eq!(map.byte_at(0, 2), 4);
    assert_eq!(map.byte_at(0, 3), 7);
}

#[test]
fn the_display_map_wraps_before_a_character_would_overflow() {
    let map = DisplayMap::new("ab漢", 3);
    let lines = map.lines();

    assert_eq!(lines.len(), 2);
    assert_eq!((lines[0].source_start, lines[0].source_end), (0, 2));
    assert!(!lines[0].wrapped);
    assert_eq!((lines[1].source_start, lines[1].source_end), (2, 5));
    assert!(lines[1].wrapped);
}

#[test]
fn a_single_wide_character_overflows_a_one_column_editor() {
    let map = DisplayMap::new("a漢", 1);

    assert_eq!(map.lines().len(), 2);
    assert_eq!(map.lines()[1].cells, 2);
}

#[test]
fn a_tab_at_the_margin_starts_a_new_visual_line() {
    let map = DisplayMap::new("abcdefgh\tx", 8);

    assert_eq!(map.lines().len(), 2);
    assert_eq!(map.lines()[0].source_end, 8);
    assert_eq!(map.lines()[1].source_start, 8);
    assert_eq!(map.lines()[1].cells, 5);
    assert!(map.lines()[1].wrapped);
}

#[test]
fn empty_logical_lines_stay_addressable() {
    let map = DisplayMap::new("a\n", 10);

    assert_eq!(map.lines().len(), 2);
    assert_eq!(map.line_of(2), 1);
    assert_eq!(map.cell_of(2), 0);
    assert_eq!(map.byte_at(1, 0), 2);
}

#[test]
fn line_and_cell_metrics_round_trip_on_every_boundary() {
    let text = "a\tbé\n漢x\n\nlast";
    let map = DisplayMap::new(text, 4);

    for (byte, _) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        let line = map.line_of(byte);
        let cell = map.cell_of(byte);
        assert_eq!(map.byte_at(line, cell), byte, "byte {byte}");
    }
}

#[test]
fn setting_the_text_places_the_caret_at_the_end_and_clears_selection() {
    let mut editor = editor("old");
    editor.select_all();

    assert!(editor.set_text("a\nrecalled command"));

    assert_eq!(editor.text(), "a\nrecalled command");
    assert_eq!(editor.caret(), editor.text().len());
    assert!(!editor.has_selection());
}

#[test]
fn setting_the_text_truncates_to_both_caps() {
    let mut bytes = Editor::new(limited(6, 3));
    assert!(bytes.set_text("abcd\nab\ncdef"));
    assert_eq!(bytes.text(), "abcd\na");

    let mut lines = Editor::new(limited(100, 2));
    assert!(lines.set_text("a\nb\nc"));
    assert_eq!(lines.text(), "a\nb");
}

#[test]
fn clearing_empties_the_buffer_and_the_caret() {
    let mut editor = editor("keep me");
    select(&mut editor, 0, 3);

    assert!(editor.clear());

    assert_eq!(editor.text(), "");
    assert_eq!(editor.caret(), 0);
    assert!(!editor.has_selection());
}

#[test]
fn take_text_drops_exactly_one_trailing_line_break() {
    let mut editor = editor("for i in 1 2 3; do\n  echo $i\ndone\n");

    assert_eq!(editor.take_text(), "for i in 1 2 3; do\n  echo $i\ndone");
    assert_eq!(editor.text(), "");
    assert_eq!(editor.caret(), 0);
}

#[test]
fn take_text_keeps_inner_breaks_and_empty_buffers() {
    let mut editor = editor("a\n\n");
    assert_eq!(editor.take_text(), "a\n");

    assert_eq!(editor.take_text(), "");
    assert_eq!(editor.take_text(), "");
}

#[test]
fn pasted_crlf_becomes_lf_and_controls_are_stripped() {
    let sanitized = sanitize_paste("a\r\nb\rc\u{1b}[31m\u{0}\t\u{7f}漢");

    assert_eq!(sanitized, "a\nb\nc[31m\t漢");
}

#[test]
fn pasting_inserts_sanitized_text_at_the_caret() {
    let mut editor = editor("> ");

    let outcome = editor.insert_paste("echo hi\r\n");

    assert!(outcome.inserted);
    assert_eq!(outcome.dropped_bytes, 0);
    assert_eq!(outcome.dropped_lines, 0);
    assert_eq!(editor.text(), "> echo hi\n");
}

#[test]
fn pasting_truncates_at_the_byte_cap_on_a_character_boundary() {
    let mut editor = Editor::new(limited(4, 512));
    editor.set_wrap_cols(20);

    let outcome = editor.insert_paste("ééé");

    assert!(outcome.inserted);
    assert_eq!(editor.text(), "éé");
    assert_eq!(editor.caret(), 4);
    assert_eq!(outcome.dropped_bytes, 2);
    assert_eq!(outcome.dropped_lines, 0);
}

#[test]
fn pasting_truncates_at_the_line_cap() {
    let mut editor = Editor::new(limited(100, 3));
    editor.set_wrap_cols(20);

    let outcome = editor.insert_paste("a\nb\nc\nd");

    assert!(outcome.inserted);
    assert_eq!(editor.text(), "a\nb\nc");
    assert_eq!(outcome.dropped_lines, 1);
    assert_eq!(outcome.dropped_bytes, 2);
}

#[test]
fn pasting_that_does_not_fit_reports_everything_dropped() {
    let mut editor = Editor::new(limited(2, 2));
    editor.set_wrap_cols(20);
    editor.set_text("ab");

    let outcome = editor.insert_paste("cd");

    assert!(!outcome.inserted);
    assert_eq!(editor.text(), "ab");
    assert_eq!(outcome.dropped_bytes, 2);
}

#[test]
fn pasting_a_selection_replacement_counts_the_freed_budget() {
    let mut editor = Editor::new(limited(4, 512));
    editor.set_wrap_cols(20);
    editor.set_text("abcd");
    editor.select_all();

    let outcome = editor.insert_paste("wxyz");

    assert!(outcome.inserted);
    assert_eq!(outcome.dropped_bytes, 0);
    assert_eq!(editor.text(), "wxyz");
}

#[test]
fn pasting_controls_only_is_a_no_op() {
    let mut editor = editor("keep");

    let outcome = editor.insert_paste("\u{1b}\u{0}\u{7}");

    assert!(!outcome.inserted);
    assert_eq!(editor.text(), "keep");
    assert_eq!(outcome.dropped_bytes, 0);
}

#[test]
fn the_byte_and_line_caps_admit_exactly_their_limit() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(200);
    let full = repeated_lines(512, 127);
    editor.set_text(&full);

    assert_eq!(editor.text(), full);
    assert_eq!(newline_count(editor.text()) + 1, 512);

    assert!(!editor.insert_text("\n").inserted);
    assert!(editor.insert_text("x").inserted);
    assert_eq!(editor.text().len(), 65_536);

    assert!(!editor.insert_text("y").inserted);
    assert_eq!(editor.text().len(), 65_536);
}

#[test]
fn the_view_hands_over_display_text_for_the_visible_window() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(10);
    editor.set_visible_lines(2);
    editor.set_text("a\nbcdefghijkl\nz");
    editor.move_to_cell(1, 0);

    let view = editor.view();

    assert_eq!(view.first_line, 1);
    assert_eq!(view.total_lines, 4);
    assert_eq!(view.lines.len(), 2);
    assert_eq!(view.lines[0].text, "bcdefghijk");
    assert_eq!(view.lines[1].text, "l");
    assert_eq!(view.lines[0].caret_cell, Some(0));
    assert_eq!(view.lines[1].caret_cell, None);
    assert_eq!(view.caret, (1, 0));
}

#[test]
fn the_view_reports_selected_cells_per_visual_line() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(4);
    editor.set_text("abcdef");
    select(&mut editor, 1, 6);

    let view = editor.line_view(0, 8);

    assert!(view.has_selection);
    assert_eq!(view.lines.len(), 2);
    assert_eq!(view.lines[0].selected_cells, Some(1..4));
    assert_eq!(view.lines[1].selected_cells, Some(0..2));
    assert_eq!(view.lines[0].selected_offsets, Some(1..4));
    assert_eq!(view.lines[1].selected_offsets, Some(0..2));
}

#[test]
fn the_view_hands_over_offsets_into_the_expanded_text() {
    let mut editor = editor("a\tb");
    editor.move_end(false);

    let view = editor.view();

    assert_eq!(view.lines[0].text, "a   b");
    assert_eq!(view.lines[0].caret_cell, Some(5));
    assert_eq!(view.lines[0].caret_offset, Some(5));

    editor.select_all();
    let view = editor.view();
    assert_eq!(view.lines[0].selected_offsets, Some(0..5));
}

#[test]
fn wide_glyphs_keep_offsets_distinct_from_cells() {
    let mut editor = editor("漢a");
    editor.move_end(false);

    let view = editor.view();

    assert_eq!(view.lines[0].text, "漢a");
    assert_eq!(view.lines[0].caret_cell, Some(3));
    assert_eq!(view.lines[0].caret_offset, Some(4));
}

#[test]
fn selected_newline_gets_a_trailing_highlight_cell() {
    let mut editor = editor("a\nb");
    select(&mut editor, 0, 3);

    let view = editor.line_view(0, 8);

    assert_eq!(view.lines[0].selected_cells, Some(0..2));
    assert_eq!(view.lines[1].selected_cells, Some(0..1));
    assert_eq!(view.lines[0].selected_offsets, Some(0..1));
    assert!(view.lines[0].newline_selected);
    assert!(!view.lines[1].newline_selected);
}

#[test]
fn the_view_expands_tabs_to_spaces() {
    let editor = editor("a\tb");

    let view = editor.view();

    assert_eq!(view.lines[0].text, "a   b");
}

#[test]
fn scrolling_moves_the_window_without_moving_the_caret() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(80);
    editor.set_visible_lines(2);
    editor.set_text("1\n2\n3\n4\n5");
    editor.move_to_cell(0, 0);
    let caret = editor.caret();

    assert!(editor.scroll_lines(2));
    assert_eq!(editor.view().first_line, 2);
    assert_eq!(editor.caret(), caret);

    assert!(editor.scroll_lines(-1));
    assert_eq!(editor.view().first_line, 1);

    assert!(editor.scroll_lines(-10));
    assert_eq!(editor.view().first_line, 0);

    assert!(!editor.scroll_lines(-1));
    assert!(editor.scroll_lines(10));
    assert_eq!(editor.view().first_line, 3);
    assert!(!editor.scroll_lines(1));
}

#[test]
fn the_window_follows_the_caret_after_an_edit() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(80);
    editor.set_visible_lines(2);
    editor.set_text("1\n2\n3\n4\n5");

    assert_eq!(editor.view().first_line, 3);

    editor.move_to_cell(0, 0);
    assert_eq!(editor.view().first_line, 0);
}

#[test]
fn visible_lines_are_clamped_to_the_row_cap() {
    let mut editor = Editor::default();

    assert!(editor.set_visible_lines(99));
    assert!(!editor.set_visible_lines(MAX_VISIBLE_LINES));
    assert!(editor.set_visible_lines(0));
    assert!(!editor.set_visible_lines(0));
}

#[test]
fn a_zero_column_wrap_behaves_as_one_column() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(0);
    editor.set_text("ab");

    assert_eq!(editor.visual_line_count(), 2);
}

#[test]
fn editing_bumps_the_revision_but_motion_does_not() {
    let mut editor = editor("abc");
    let revision = editor.revision();

    editor.move_to_cell(0, 1);
    editor.move_vertical(Direction::Down, false);
    assert_eq!(editor.revision(), revision);

    editor.insert_text("x");
    assert!(editor.revision() > revision);
}

#[test]
fn utf8_boundaries_survive_a_mixed_edit_sequence() {
    let mut editor = editor("");

    let alphabet = ['a', 'é', '漢', '🙂', '\n', '\t'];
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    for _ in 0..200 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let ch = alphabet[(state >> 33) as usize % alphabet.len()];
        editor.insert_text(&ch.to_string());
        editor.delete_backward();
        editor.move_horizontal((state % 5) as isize - 2, false);
        editor.move_vertical(Direction::Down, false);
        editor.move_to_cell((state % 4) as usize, (state % 9) as usize);

        assert!(editor.text().is_char_boundary(editor.caret()));
        if let Some(range) = editor.selection_range() {
            assert!(editor.text().is_char_boundary(range.start));
            assert!(editor.text().is_char_boundary(range.end));
        }
    }
}

#[test]
fn the_view_window_never_exceeds_the_line_cap() {
    let mut editor = Editor::new(EditorLimits::default());
    editor.set_wrap_cols(2);
    editor.set_visible_lines(8);
    editor.set_text("abcdefghij\nklm");

    for delta in [-100, -3, 0, 2, 100] {
        editor.scroll_lines(delta);
        let view = editor.view();
        assert!(view.first_line + view.lines.len() <= view.total_lines);
        assert!(view.lines.len() <= MAX_VISIBLE_LINES);
    }
}
