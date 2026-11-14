//! OSC dispatch: 133 markers, 633 payloads, and marker/row interleaving.

use super::super::OSC_MAX_BYTES;
use super::*;

#[test]
fn prompt_markers_are_classified_in_order() {
    let items = items_of(&[b"\x1b]133;A\x1b\\\x1b]133;B\x1b\\\x1b]133;C\x1b\\"]);

    assert_eq!(
        items,
        vec![
            semantic(SemanticEvent::PromptStart),
            semantic(SemanticEvent::PromptEnd),
            semantic(SemanticEvent::CommandStart),
        ]
    );
}

#[test]
fn command_finished_parses_bel_terminated_exit_codes() {
    let items = items_of(&[b"\x1b]133;D;0\x07\x1b]133;D;130\x1b\\"]);

    assert_eq!(
        items,
        vec![
            semantic(SemanticEvent::CommandFinished { exit_code: 0 }),
            semantic(SemanticEvent::CommandFinished { exit_code: 130 }),
        ]
    );
}

#[test]
fn malformed_command_finished_markers_are_ignored() {
    let payloads: &[&[u8]] = &[
        b"\x1b]133;D\x1b\\",
        b"\x1b]133;D;\x1b\\",
        b"\x1b]133;D;abc\x1b\\",
        b"\x1b]133;D;0;extra\x1b\\",
    ];

    for payload in payloads {
        assert!(items_of(&[payload]).is_empty(), "payload {payload:?}");
    }
}

#[test]
fn prompt_markers_with_extra_parameters_are_ignored() {
    let payloads: &[&[u8]] = &[
        b"\x1b]133;A;x\x1b\\",
        b"\x1b]133;B;x\x1b\\",
        b"\x1b]133;C;x\x1b\\",
    ];

    for payload in payloads {
        assert!(items_of(&[payload]).is_empty(), "payload {payload:?}");
    }
}

#[test]
fn cwd_payload_is_take_rest() {
    let items = items_of(&[b"\x1b]633;P;Cwd=/tmp/100%;a=b\x1b\\"]);

    assert_eq!(
        items,
        vec![semantic(SemanticEvent::DirectoryChanged {
            path: PathBuf::from("/tmp/100%;a=b"),
        })]
    );
}

#[test]
fn empty_or_non_cwd_payloads_are_ignored() {
    assert!(items_of(&[b"\x1b]633;P;Cwd=\x1b\\"]).is_empty());
    assert!(items_of(&[b"\x1b]633;P;NotCwd=/tmp\x1b\\"]).is_empty());
}

#[test]
fn command_line_payload_is_take_rest() {
    let items = items_of(&[b"\x1b]633;E;printf '100%n'; ls -la\x1b\\"]);

    assert_eq!(
        items,
        vec![semantic(SemanticEvent::CommandLine {
            text: "printf '100%n'; ls -la".into(),
        })]
    );
}

#[test]
fn command_line_drops_control_bytes_and_keeps_text() {
    // vte discards C0 bytes inside OSC, so the embedded newline is lost
    // rather than printed or turned into a header line.
    let items = items_of(&[b"\x1b]633;E;echo a\nb\x7fc\x1b\\"]);

    assert_eq!(
        items,
        vec![semantic(SemanticEvent::CommandLine {
            text: "echo abc".into(),
        })]
    );
}

#[test]
fn empty_command_line_is_ignored() {
    assert!(items_of(&[b"\x1b]633;E;\x1b\\"]).is_empty());
}

#[test]
fn bare_and_unknown_osc_payloads_are_ignored() {
    assert!(items_of(&[b"\x1b]133;P\x1b\\"]).is_empty());
    assert!(items_of(&[b"\x1b]0;title\x07"]).is_empty());
}

#[test]
fn a_written_row_precedes_the_marker_that_interrupts_it() {
    let items = items_of(&[b"prompt$ \x1b]133;C\x1b\\"]);

    assert_eq!(items.len(), 2);
    let SurfaceItem::Row(row) = &items[0] else {
        panic!("expected the prompt row first");
    };
    assert_eq!(text(row), "prompt$ ");
    assert_eq!(items[1], semantic(SemanticEvent::CommandStart));
}

#[test]
fn output_after_a_marker_starts_at_column_zero() {
    let items = items_of(&[b"prompt$ \x1b]133;C\x1b\\\x1b[31mout\x1b[0m\r\n"]);

    let rows: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            SurfaceItem::Row(row) => Some(text(row)),
            SurfaceItem::Semantic(_) => None,
        })
        .collect();
    assert_eq!(rows, ["prompt$ ", "out"]);
}

#[test]
fn zle_line_clear_between_c_and_d_is_not_an_output_row() {
    // ZLE paints an inverse `%`, pads the line, then erases it with CR
    // before precmd emits `D`. The erased row must not reach the block.
    let mut bytes =
        Vec::from(&b"prompt$ \x1b]133;C\x1b\\\x1b[1m\x1b[7m%\x1b[27m\x1b[1m\x1b[0m"[..]);
    bytes.extend(std::iter::repeat_n(b' ', 79));
    bytes.extend_from_slice(b"\r \r\x1b]133;D;1\x1b\\");
    let items = items_of(&[bytes.as_slice()]);

    let rows: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            SurfaceItem::Row(row) => Some(text(row)),
            SurfaceItem::Semantic(_) => None,
        })
        .collect();
    assert_eq!(rows, ["prompt$ "]);
    assert!(items.contains(&semantic(SemanticEvent::CommandFinished { exit_code: 1 })));
}

#[test]
fn command_finished_commits_a_partial_output_row() {
    let items = items_of(&[b"result\x1b]133;D;0\x1b\\"]);

    assert_eq!(items.len(), 2);
    let SurfaceItem::Row(row) = &items[0] else {
        panic!("expected the output row first");
    };
    assert_eq!(text(row), "result");
    assert_eq!(
        items[1],
        semantic(SemanticEvent::CommandFinished { exit_code: 0 })
    );
}

#[test]
fn terminal_reset_flushes_output_and_emits_reset() {
    let items = items_of(&[b"tail\x1bc"]);

    assert_eq!(items.len(), 2);
    let SurfaceItem::Row(row) = &items[0] else {
        panic!("expected the output row first");
    };
    assert_eq!(text(row), "tail");
    assert_eq!(items[1], semantic(SemanticEvent::TerminalReset));
}

#[test]
fn markers_split_across_chunks_reassemble() {
    let items = items_of(&[b"\x1b]133;", b"D;7\x1b\\"]);

    assert_eq!(
        items,
        vec![semantic(SemanticEvent::CommandFinished { exit_code: 7 })]
    );
}

#[test]
fn command_line_truncates_at_the_vte_parameter_cap() {
    let fields: Vec<String> = (0..20).map(|index| format!("p{index}")).collect();
    let command = fields.join(";");
    let payload = format!("\x1b]633;E;{command}\x1b\\");

    let items = items_of(&[payload.as_bytes()]);

    // vte keeps at most 16 semicolon-separated parameters: 633, E and 14
    // payload fields. The remaining command tail is a documented loss.
    let captured = fields[..14].join(";");
    assert_eq!(
        items,
        vec![semantic(SemanticEvent::CommandLine { text: captured })]
    );
}

#[test]
fn an_unrecognized_esc_sequence_is_ignored() {
    // DECALN stays outside the subset; only RIS (ESC c) is semantic.
    assert!(items_of(&[b"\x1b#8"]).is_empty());
}

#[test]
fn unterminated_osc_capture_stays_bounded() {
    let mut payload = Vec::from(b"\x1b]633;E;".as_slice());
    payload.extend(std::iter::repeat_n(b'a', OSC_MAX_BYTES * 2));
    payload.extend_from_slice(b"\x1b\\");

    let items = items_of(&[payload.as_slice()]);

    let [SurfaceItem::Semantic(SemanticEvent::CommandLine { text })] = items.as_slice() else {
        panic!("expected a truncated command line, got {items:?}");
    };
    assert!(text.len() <= OSC_MAX_BYTES);
    assert!(text.bytes().all(|byte| byte == b'a'));
}
