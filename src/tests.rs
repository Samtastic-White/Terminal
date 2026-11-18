//! End-to-end wiring tests: session service against the real writer and, where
//! noted, the real PTY adapter.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::application::session::{EditCommand, EditorState, SessionService, SubmitOutcome};
use crate::domain::block::{Block, BlockStatus, CopyKind};
use crate::domain::output::{Color, Row};
use crate::domain::pty::PtyPort;
use crate::domain::semantic::ShellPhase;
use crate::domain::size::TerminalSize;
use crate::infrastructure::parser::BlockTextWriter;
use crate::infrastructure::pty::PtyAdapter;
use crate::probe::measure_enabled_from;

fn row_text(row: &Row) -> String {
    row.spans.iter().map(|span| span.text.as_str()).collect()
}

fn output_text(block: &Block) -> Vec<String> {
    block.output().iter().map(row_text).collect()
}

fn output_contains(service: &SessionService, needle: &str) -> bool {
    service.blocks().iter().any(|block| {
        block
            .output()
            .iter()
            .any(|row| row.spans.iter().any(|span| span.text.contains(needle)))
    })
}

fn red_output_present(service: &SessionService) -> bool {
    service.blocks().iter().any(|block| {
        block.output().iter().any(|row| {
            row.spans
                .iter()
                .any(|span| span.text == "RED" && span.style.fg == Color::Indexed(1))
        })
    })
}

struct NullPty;

impl PtyPort for NullPty {
    fn spawn(&mut self, _size: TerminalSize) -> crate::common::errors::Result<()> {
        Ok(())
    }

    fn write(&mut self, _bytes: &[u8]) -> crate::common::errors::Result<()> {
        Ok(())
    }

    fn resize(&mut self, _size: TerminalSize) -> crate::common::errors::Result<()> {
        Ok(())
    }

    fn is_alive(&self) -> bool {
        true
    }
}

fn drain_until_editor_idle(
    receiver: &mut Receiver<Vec<u8>>,
    service: &mut SessionService,
    deadline: Duration,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        let remaining = deadline.saturating_sub(start.elapsed());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(chunk) => service.ingest_bytes(&chunk),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
        if service.editor_state() == EditorState::Idle {
            return true;
        }
    }
    false
}

fn drain_until_block_done(
    receiver: &mut Receiver<Vec<u8>>,
    service: &mut SessionService,
    target_blocks: usize,
    deadline: Duration,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        let remaining = deadline.saturating_sub(start.elapsed());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(chunk) => service.ingest_bytes(&chunk),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if service.blocks().len() >= target_blocks
            && service
                .blocks()
                .last()
                .map(|block| !block.is_running())
                .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn drain_for(receiver: &mut Receiver<Vec<u8>>, service: &mut SessionService, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
            Ok(chunk) => service.ingest_bytes(&chunk),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn spawn_service() -> (Receiver<Vec<u8>>, SessionService) {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let receiver = adapter.take_output().unwrap();
    let service = SessionService::new(Box::new(adapter), Box::new(BlockTextWriter::new(80)));
    (receiver, service)
}

fn feedback_text(service: &SessionService) -> Vec<String> {
    service.feedback_rows().iter().map(row_text).collect()
}

#[test]
fn measure_flag_accepts_only_one() {
    assert!(measure_enabled_from(Some("1")));
    assert!(!measure_enabled_from(Some("0")));
    assert!(!measure_enabled_from(Some("yes")));
    assert!(!measure_enabled_from(None));
}

#[test]
fn raw_bytes_flow_into_a_completed_block() {
    let mut service = SessionService::new(Box::new(NullPty), Box::new(BlockTextWriter::new(80)));
    service.ingest_bytes(b"\x1b]133;A\x1b\\\x1b[32muser@host:~$ \x1b[0m\x1b]133;B\x1b\\");
    service.ingest_bytes(b"\x1b]133;C\x1b\\\x1b]633;E;echo RED\x1b\\");
    service.ingest_bytes(b"\x1b[31mRED\x1b[0m\r\n\xe4\xbd\xa0\xe5\xa5\xbd\r\n\x1b]133;D;0\x1b\\");

    assert_eq!(service.blocks().len(), 1);
    let block = &service.blocks()[0];
    assert_eq!(block.command(), Some("echo RED"));
    assert_eq!(block.status(), BlockStatus::Succeeded);
    assert_eq!(output_text(block), ["RED", "你好"]);
    assert!(red_output_present(&service));
}

#[test]
fn live_row_renders_only_inside_a_running_block() {
    let mut service = SessionService::new(Box::new(NullPty), Box::new(BlockTextWriter::new(80)));

    service.ingest_bytes(b"\x1b]133;A\x1b\\");
    assert!(service.blocks().is_empty());
    assert!(service.live_row().is_none());

    service.ingest_bytes(b"\x1b]133;B\x1b\\\x1b]133;C\x1b\\");
    assert_eq!(service.blocks().len(), 1);

    service.ingest_bytes(b"next");
    assert_eq!(
        row_text(&service.live_row().expect("running output must be live")),
        "next"
    );
    assert!(service.blocks()[0].output().is_empty());

    service.ingest_bytes(b"\r\n");
    assert!(service.live_row().is_none());
    assert_eq!(output_text(&service.blocks()[0]), ["next"]);
}

#[test]
fn default_block_cap_bounds_real_output() {
    let mut service = SessionService::new(Box::new(NullPty), Box::new(BlockTextWriter::new(80)));
    service.ingest_bytes(b"\x1b]133;A\x1b\\\x1b]133;C\x1b\\");

    for n in 0..4_010 {
        service.ingest_bytes(format!("{n}\r\n").as_bytes());
    }

    let block = &service.blocks()[0];
    assert_eq!(block.output().len(), 4_000);
    assert_eq!(
        row_text(block.output().iter().next().expect("non-empty output")),
        "10"
    );
    assert_eq!(
        row_text(block.output().iter().next_back().expect("non-empty output")),
        "4009"
    );
}

#[test]
fn engine_end_to_end_renders_colors_cjk_progress_and_resize() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();
    let mut service = SessionService::new(Box::new(adapter), Box::new(BlockTextWriter::new(80)));

    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the injected prompt marker must open the prompt window"
    );

    let target = service.blocks().len() + 1;
    service
        .write_keystrokes(b"printf '\\033[31mRED\\033[0m\\n'\r")
        .unwrap();
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(5)),
        "ANSI command must complete"
    );
    assert!(
        red_output_present(&service),
        "ANSI red output must reach a block"
    );
    assert_eq!(
        service.blocks().last().expect("ANSI block").status(),
        BlockStatus::Succeeded
    );

    let target = service.blocks().len() + 1;
    service
        .write_keystrokes("printf '\\344\\275\\240\\345\\245\\275\\n'\r".as_bytes())
        .unwrap();
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(5)),
        "CJK command must complete"
    );
    assert!(
        output_contains(&service, "你好"),
        "CJK output must reach a block"
    );

    let target = service.blocks().len() + 1;
    service
        .write_keystrokes(b"printf '50%%\\r75%%\\n'\r")
        .unwrap();
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(5)),
        "progress command must complete"
    );
    assert!(
        output_contains(&service, "75%"),
        "carriage-return rewrite must reach a block"
    );

    service.set_editor_geometry(80, 3);
    assert_eq!(
        service.editor_visual_lines(),
        3,
        "the dock keeps a three-line editor while resizing"
    );
    service.edit(EditCommand::Insert("one\ntwo\nthree".to_owned()));
    assert_eq!(service.editor().visual_line_count(), 3);

    service.resize(TerminalSize::new(100, 40)).unwrap();
    service.edit(EditCommand::Clear);
    service.edit(EditCommand::Insert("stty size".to_owned()));
    let target = service.blocks().len() + 1;
    assert_eq!(service.submit(), SubmitOutcome::Submitted);
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(5)),
        "stty size must complete"
    );
    assert!(
        output_contains(&service, "40 100"),
        "SIGWINCH must resize the child to 100x40"
    );

    assert!(
        !output_contains(&service, "]133") && !output_contains(&service, "]633"),
        "semantic markers must never render as text"
    );
}

#[test]
fn empty_enter_and_ctrl_c_at_the_prompt_create_no_blocks() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();
    let mut service = SessionService::new(Box::new(adapter), Box::new(BlockTextWriter::new(80)));

    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the shell must reach its first prompt"
    );
    let before = service.blocks().len();

    assert_eq!(
        service.submit(),
        SubmitOutcome::Ignored,
        "an empty Enter must not write to the PTY"
    );
    drain_for(&mut receiver, &mut service, Duration::from_millis(400));
    assert_eq!(
        service.blocks().len(),
        before,
        "an empty Enter must not open a block"
    );

    service.interrupt();
    drain_for(&mut receiver, &mut service, Duration::from_millis(400));
    assert_eq!(
        service.blocks().len(),
        before,
        "Ctrl+C at the prompt must not open a block"
    );
    assert!(
        !matches!(service.phase(), ShellPhase::Running),
        "Ctrl+C at the prompt must leave the shell prompting"
    );
}

#[test]
fn failing_command_records_the_exit_status() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();
    let mut service = SessionService::new(Box::new(adapter), Box::new(BlockTextWriter::new(80)));

    assert!(drain_until_editor_idle(
        &mut receiver,
        &mut service,
        Duration::from_secs(5)
    ));
    let target = service.blocks().len() + 1;
    service.edit(EditCommand::Insert("false".to_owned()));
    assert_eq!(service.submit(), SubmitOutcome::Submitted);
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(5)),
        "false must complete"
    );

    let block = service.blocks().last().expect("one completed block");
    assert_eq!(block.status(), BlockStatus::Failed { exit_code: 1 });
    assert!(
        block.output().is_empty(),
        "false must keep an empty block body, got {:?}",
        output_text(block)
    );
}

#[test]
fn multiline_submission_runs_once_and_keeps_the_full_command() {
    let (mut receiver, mut service) = spawn_service();
    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the shell must reach its first prompt"
    );
    assert!(service.blocks().is_empty());
    assert!(service.live_row().is_none());

    let command = "for i in 1 2 3\ndo\necho \"line $i\"\ndone";
    service.edit(EditCommand::Insert(command.to_owned()));
    assert_eq!(service.submit(), SubmitOutcome::Submitted);
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);
    assert!(service.editor().text().is_empty());

    let target = service.blocks().len() + 1;
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(5)),
        "the multiline command must complete"
    );
    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the next prompt must unlock the editor"
    );

    let block = service.blocks().last().expect("one completed block");
    assert_eq!(block.status(), BlockStatus::Succeeded);
    assert_eq!(block.command(), Some(command));
    assert_eq!(
        output_text(block),
        ["line 1", "line 2", "line 3"],
        "no prompt or submitted bytes may leak into the body"
    );
    assert_eq!(
        service.copy_block(block.id(), CopyKind::Command).as_deref(),
        Some(command),
        "the command copy keeps every line break"
    );
    assert_eq!(service.blocks().len(), target);
    assert_eq!(service.editor_state(), EditorState::Idle);
}

#[test]
fn syntax_error_shows_feedback_and_restores_the_text() {
    let (mut receiver, mut service) = spawn_service();
    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the shell must reach its first prompt"
    );

    let command = "echo )";
    service.edit(EditCommand::Insert(command.to_owned()));
    assert_eq!(service.submit(), SubmitOutcome::Submitted);

    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the parse error must return to a prompt"
    );

    assert!(
        service.blocks().is_empty(),
        "a parse error must not open a block"
    );
    assert_eq!(
        service.editor().text(),
        command,
        "the pending text must be restored after D without C"
    );
    assert!(
        feedback_text(&service)
            .iter()
            .any(|row| row.contains("parse error")),
        "the shell error must stay as feedback, got {:?}",
        feedback_text(&service)
    );
}

#[test]
fn ctrl_c_restores_a_command_stuck_at_a_continuation_prompt() {
    let (mut receiver, mut service) = spawn_service();
    assert!(
        drain_until_editor_idle(&mut receiver, &mut service, Duration::from_secs(5)),
        "the shell must reach its first prompt"
    );

    let command = "for i in 1 2 3";
    service.edit(EditCommand::Insert(command.to_owned()));
    assert_eq!(service.submit(), SubmitOutcome::Submitted);

    // Let zsh consume the line and wait for the loop body.
    drain_for(&mut receiver, &mut service, Duration::from_millis(300));
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);

    service.interrupt();
    assert_eq!(
        service.editor().text(),
        command,
        "Ctrl+C must hand the pending text back immediately"
    );

    drain_for(&mut receiver, &mut service, Duration::from_millis(500));
    assert_eq!(service.editor().text(), command);
    assert!(
        service.blocks().is_empty(),
        "the aborted command must not open a block"
    );
}

#[test]
fn nested_zsh_does_not_create_a_nested_block() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();
    let mut service = SessionService::new(Box::new(adapter), Box::new(BlockTextWriter::new(80)));

    assert!(drain_until_editor_idle(
        &mut receiver,
        &mut service,
        Duration::from_secs(5)
    ));
    let target = service.blocks().len() + 1;
    service
        .write_keystrokes(b"zsh -ic 'echo NESTED_OK'\r")
        .unwrap();
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(10)),
        "nested zsh must complete in the outer block"
    );

    assert_eq!(
        service.blocks().len(),
        target,
        "the nesting guard must suppress nested cards"
    );
    let block = service.blocks().last().expect("one completed block");
    assert_eq!(block.status(), BlockStatus::Succeeded);
    assert!(output_contains(&service, "NESTED_OK"));
    assert!(
        !output_contains(&service, "]133") && !output_contains(&service, "]633"),
        "nested markers must never render as text"
    );
}

#[test]
fn streaming_output_keeps_the_block_bounded() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();
    let mut service = SessionService::new(Box::new(adapter), Box::new(BlockTextWriter::new(80)));

    assert!(drain_until_editor_idle(
        &mut receiver,
        &mut service,
        Duration::from_secs(5)
    ));
    let target = service.blocks().len() + 1;
    service.write_keystrokes(b"seq 1 5000\r").unwrap();
    assert!(
        drain_until_block_done(&mut receiver, &mut service, target, Duration::from_secs(10)),
        "seq must complete"
    );

    let block = service.blocks().last().expect("one completed block");
    assert_eq!(block.status(), BlockStatus::Succeeded);
    assert_eq!(
        block.output().len(),
        4_000,
        "output rows must stay capped, got {:?}…{:?}",
        block.output().iter().next().map(row_text),
        block.output().iter().next_back().map(row_text)
    );
    assert_eq!(
        row_text(block.output().iter().next().expect("bounded output")),
        "1001"
    );
    assert_eq!(
        row_text(block.output().iter().next_back().expect("bounded output")),
        "5000"
    );
}
