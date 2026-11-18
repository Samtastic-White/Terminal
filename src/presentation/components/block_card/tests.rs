//! Block card unit tests: body flattening, slicing, snapshots, and copy
//! extraction.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::domain::block::{Block, BlockId, CopyKind};
use crate::domain::output::{Color, Row, Span, TextStyle};
use crate::presentation::theme::TerminalTheme;

use super::{
    BlockUiState, CARD_ACTIONS, CardSnapshot, StatusTone, build_body_text, card_snapshot,
    clamp_selection, command_preview, status_label, status_tone,
};

fn row(text: &str) -> Row {
    Row {
        spans: vec![Span::new(TextStyle::default(), text)],
    }
}

fn styled_row(parts: &[(&str, TextStyle)]) -> Row {
    Row {
        spans: parts
            .iter()
            .map(|(text, style)| Span::new(*style, *text))
            .collect(),
    }
}

fn block(id: u64, started_mono: Instant) -> Block {
    Block::running(
        BlockId::new(id),
        Some(PathBuf::from("/tmp")),
        SystemTime::UNIX_EPOCH,
        started_mono,
        16,
    )
}

#[test]
fn body_text_flattens_spans_with_newlines_between_rows() {
    let bold = TextStyle {
        bold: true,
        fg: Color::Indexed(1),
        ..TextStyle::default()
    };

    let body = build_body_text(&[
        styled_row(&[("hi", bold), (" there", TextStyle::default())]),
        row("next"),
    ]);

    assert_eq!(body.text(), "hi there\nnext");
    assert_eq!(body.row_count(), 2);
    assert_eq!(body.runs().len(), 2);
    assert_eq!(body.runs()[0].range(), &(0..2));
    assert!(body.runs()[0].style().bold);
    // Newline separators merge into adjacent default-styled runs.
    assert_eq!(body.runs()[1].range(), &(2..13));
}

#[test]
fn blank_rows_keep_their_stream_position() {
    let body = build_body_text(&[row("ab"), Row::default(), row("cd")]);

    assert_eq!(body.text(), "ab\n\ncd");
    assert_eq!(body.row_count(), 3);
    assert_eq!(body.slice(0, 2, None).text(), "ab\n");
    assert_eq!(body.slice(1, 3, None).text(), "\ncd");
    assert_eq!(body.slice(1, 2, None).text(), "");
}

#[test]
fn a_trailing_blank_row_stays_addressable() {
    let body = build_body_text(&[row("ab"), Row::default()]);

    assert_eq!(body.text(), "ab\n");
    assert_eq!(body.row_count(), 2);
    assert_eq!(body.slice(1, 2, None).text(), "");
    assert_eq!(body.slice(0, 2, None).text(), "ab\n");
}

#[test]
fn incremental_append_matches_a_fresh_build() {
    let mut body = build_body_text(&[row("a"), Row::default()]);
    body.push_row(&row("c"));
    body.push_row(&row("d"));

    let fresh = build_body_text(&[row("a"), Row::default(), row("c"), row("d")]);
    assert_eq!(body.text(), fresh.text());
    assert_eq!(body.row_count(), fresh.row_count());
    assert_eq!(body.runs(), fresh.runs());
    assert_eq!(body.slice(0, 4, None), fresh.slice(0, 4, None));
}

#[test]
fn front_drop_keeps_offsets_relative_without_compaction() {
    let mut body = build_body_text(&[row("a"), row("b"), row("c")]);
    body.drop_front(1);

    let fresh = build_body_text(&[row("b"), row("c")]);
    assert_eq!(body.row_count(), 2);
    assert_eq!(body.slice(0, 2, None), fresh.slice(0, 2, None));
    assert_eq!(body.slice(0, 2, None).start_byte(), 0);
}

#[test]
fn front_drop_reclaims_evicted_bytes_once_they_dominate() {
    let long = "x".repeat(400);
    let rows: Vec<Row> = (0..300).map(|_| row(&long)).collect();
    let mut body = build_body_text(rows.iter());
    body.drop_front(200);

    let fresh = build_body_text(rows.iter().skip(200));
    assert_eq!(body.row_count(), 100);
    assert_eq!(body.text(), fresh.text());
    assert_eq!(body.slice(10, 20, None), fresh.slice(10, 20, None));
}

#[test]
fn selection_splits_runs_and_keeps_styles() {
    let bold = TextStyle {
        bold: true,
        ..TextStyle::default()
    };
    let body = build_body_text(&[styled_row(&[("abcdef", bold)])]);

    let slice = body.slice(0, 1, Some(2..4));

    assert_eq!(slice.text(), "abcdef");
    assert_eq!(slice.runs().len(), 3);
    assert!(!slice.runs()[0].selected());
    assert!(slice.runs()[1].selected());
    assert!(!slice.runs()[2].selected());
    assert_eq!(&slice.text()[slice.runs()[1].range().clone()], "cd");
    assert_eq!(slice.selected_text(), "cd");
    assert!(slice.runs().iter().all(|run| run.style().bold));
}

#[test]
fn selection_survives_scrolling_between_row_windows() {
    let rows: Vec<Row> = (0..40).map(|index| row(&format!("line {index}"))).collect();
    let body = build_body_text(&rows);
    let start = body.text().find("line 32").expect("fixture row");
    let end = body.text().find("line 36").expect("fixture row") + "line 36".len();
    let selection = start..end;

    let head = body.slice(0, 28, Some(selection.clone()));
    assert!(head.selected_text().is_empty());

    let tail = body.slice(12, 40, Some(selection));
    let selected = tail.selected_text();
    assert!(selected.contains("line 32"));
    assert!(selected.contains("line 35"));
}

#[test]
fn selection_clamps_to_utf8_boundaries() {
    let text = "año🙂";
    // ñ is 1..3 bytes, the emoji is 4..8.
    assert_eq!(clamp_selection(2..5, text), 1..8);
    // 0..1 ends exactly at the start of ñ: a valid single-character range.
    assert_eq!(clamp_selection(0..1, text), 0..1);
    // 2..4 starts inside ñ and ends at the emoji boundary.
    assert_eq!(clamp_selection(2..4, text), 1..4);
    assert_eq!(clamp_selection(6..99, text), 4..8);
}

#[test]
fn slicing_never_splits_a_multibyte_glyph() {
    let body = build_body_text(&[row("año🙂")]);
    let slice = body.slice(0, 1, Some(2..5));

    assert_eq!(slice.selected_text(), "ño🙂");
    for run in slice.runs() {
        assert!(slice.text().is_char_boundary(run.range().start));
        assert!(slice.text().is_char_boundary(run.range().end));
    }
}

#[test]
fn state_pills_cover_every_status() {
    let mono = Instant::now();
    let mut running = block(1, mono);
    assert_eq!(status_label(running.status()), "running");
    assert_eq!(status_tone(running.status()), StatusTone::Running);

    running.complete(0, SystemTime::UNIX_EPOCH, mono);
    assert_eq!(status_label(running.status()), "ok");
    assert_eq!(status_tone(running.status()), StatusTone::Success);

    let mut failed = block(2, mono);
    failed.complete(2, SystemTime::UNIX_EPOCH, mono);
    assert_eq!(status_label(failed.status()), "exit 2");
    assert_eq!(status_tone(failed.status()), StatusTone::Failure);

    let mut interrupted = block(3, mono);
    interrupted.interrupt(SystemTime::UNIX_EPOCH, mono);
    assert_eq!(status_label(interrupted.status()), "—");
    assert_eq!(status_tone(interrupted.status()), StatusTone::Interrupted);
}

#[test]
fn snapshot_reports_command_relative_time_and_duration() {
    let mono = Instant::now();
    let mut block = block(1, mono);
    block.set_command("echo hi".into());
    block.push_row(row("hi"));
    block.complete(
        0,
        SystemTime::UNIX_EPOCH + Duration::from_secs(125),
        mono + Duration::from_millis(1_500),
    );

    let snapshot = card_snapshot(&block, SystemTime::UNIX_EPOCH + Duration::from_secs(125));

    assert_eq!(snapshot.command, "echo hi");
    assert_eq!(snapshot.status_label, "ok");
    assert_eq!(snapshot.tone, StatusTone::Success);
    assert_eq!(snapshot.meta, "2 min ago · 1.50s");
    assert!(snapshot.has_body);
}

#[test]
fn running_snapshot_has_no_duration_and_empty_output_has_no_body() {
    let mono = Instant::now();
    let running = block(1, mono);

    let snapshot = card_snapshot(&running, SystemTime::UNIX_EPOCH);

    assert_eq!(snapshot.command, "…");
    assert_eq!(snapshot.status_label, "running");
    assert_eq!(snapshot.meta, "just now");
    assert!(!snapshot.has_body);
}

#[test]
fn command_preview_keeps_only_the_first_logical_line() {
    let mono = Instant::now();
    let mut block = block(1, mono);
    let command = "for x in 1 2\ndo\necho $x\ndone";
    block.set_command(command.into());

    assert_eq!(command_preview(&block), "for x in 1 2…");
    assert_eq!(block.copy(CopyKind::Command), command);
}

#[test]
fn live_row_appends_after_the_committed_slice_and_is_never_selected() {
    let body = build_body_text(&[row("done")]);
    let mut slice = body.slice(0, 1, Some(0..4));

    slice.push_live_row(&row("live"));

    assert_eq!(slice.text(), "done\nlive");
    assert_eq!(slice.selected_text(), "done");
    assert!(
        slice
            .runs()
            .iter()
            .filter(|run| run.selected())
            .all(|run| run.range().end <= 4)
    );

    let blank = build_body_text(&[Row::default()]);
    let mut blank_slice = blank.slice(0, 1, None);
    blank_slice.push_live_row(&row("live"));
    assert_eq!(blank_slice.text(), "\nlive");
}

#[test]
fn body_selection_text_spans_rows_and_starts_on_boundaries() {
    let body = build_body_text(&[row("one"), row("two")]);

    assert_eq!(body.selected_text(1..6), "ne\ntw");
    assert_eq!(body.selected_text(4..7), "two");
}

#[test]
fn a_live_row_shapes_a_body_even_before_the_first_committed_row() {
    let mono = Instant::now();
    let empty = block(2, mono);
    let mut block = block(1, mono);
    block.push_row(row("done"));
    let mut ui = BlockUiState::new();
    ui.sync_body(&block);

    let live = row("in progress");
    assert!(ui.set_live(Some(&live)));
    assert!(ui.prepare_body(&TerminalTheme::default()).is_some());
    assert_eq!(
        ui.pending.as_ref().map(|pending| pending.text.as_str()),
        Some("done\nin progress")
    );
    assert!(!ui.set_live(Some(&live)));

    let mut ui = BlockUiState::new();
    ui.sync_body(&empty);
    assert!(ui.set_live(Some(&live)));
    assert!(ui.prepare_body(&TerminalTheme::default()).is_some());
    assert_eq!(
        ui.pending.as_ref().map(|pending| pending.text.as_str()),
        Some("in progress")
    );
}

#[test]
fn copy_actions_extract_each_surface_verbatim() {
    let mono = Instant::now();
    let mut block = block(1, mono);
    block.set_command("echo hi".into());
    block.push_row(row("hi"));

    let labels: Vec<&str> = CARD_ACTIONS.iter().map(|action| action.label).collect();
    assert_eq!(labels, ["Copy", "Cmd", "Out"]);

    let copied: Vec<String> = CARD_ACTIONS
        .iter()
        .map(|action| block.copy(action.kind))
        .collect();
    assert_eq!(copied, ["echo hi\nhi", "echo hi", "hi"]);
}

#[test]
fn card_snapshots_build_for_zero_one_and_many_blocks() {
    let mono = Instant::now();
    let now = SystemTime::UNIX_EPOCH;
    let one = [block(1, mono)];
    let many = [block(1, mono), block(2, mono), block(3, mono)];

    let snapshots = |blocks: &[Block]| -> Vec<CardSnapshot> {
        blocks
            .iter()
            .map(|block| card_snapshot(block, now))
            .collect()
    };

    assert!(snapshots(&[]).is_empty());
    assert_eq!(snapshots(&one).len(), 1);
    assert_eq!(snapshots(&many).len(), 3);
}
