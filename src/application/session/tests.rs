//! Session service unit tests: editor lifecycle, submission, history, and
//! routing through the mock PTY and surface ports.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::{EditCommand, EditorState, SessionService, SubmitOutcome};
use crate::domain::block::{BlockId, BlockStatus, CopyKind};
use crate::domain::keymap::{KeyChord, KeyToken, NamedKey, TerminalModes};
use crate::domain::output::{Row, Span, TextStyle};
use crate::domain::pty::PtyPort;
use crate::domain::semantic::{SemanticEvent, ShellPhase};
use crate::domain::size::TerminalSize;
use crate::domain::terminal_surface::{SurfaceItem, TerminalSurfacePort};

/// Recorded `PtyPort` calls shared with the test through `Rc`.
#[derive(Default, Debug)]
struct PtyRecorder {
    sizes: Vec<TerminalSize>,
    writes: Vec<Vec<u8>>,
    fail_writes: bool,
}

type PtyShared = Rc<RefCell<PtyRecorder>>;

struct MockPty {
    rec: PtyShared,
}

impl MockPty {
    fn new() -> (Self, PtyShared) {
        let rec: PtyShared = Rc::new(RefCell::new(PtyRecorder::default()));
        (Self { rec: rec.clone() }, rec)
    }
}

impl PtyPort for MockPty {
    fn spawn(&mut self, _size: TerminalSize) -> crate::common::errors::Result<()> {
        Ok(())
    }
    fn write(&mut self, bytes: &[u8]) -> crate::common::errors::Result<()> {
        let mut rec = self.rec.borrow_mut();
        if rec.fail_writes {
            return Err(crate::common::errors::TerminalError::Pty(
                "write failed".into(),
            ));
        }
        rec.writes.push(bytes.to_vec());
        Ok(())
    }
    fn resize(&mut self, size: TerminalSize) -> crate::common::errors::Result<()> {
        self.rec.borrow_mut().sizes.push(size);
        Ok(())
    }
    fn is_alive(&self) -> bool {
        true
    }
}

#[derive(Default, Debug)]
struct InterpRecorder {
    pending: Vec<SurfaceItem>,
    active: Option<Row>,
    interpreted: Vec<Vec<u8>>,
    resized: Vec<TerminalSize>,
    modes: TerminalModes,
}

type InterpShared = Rc<RefCell<InterpRecorder>>;

struct EchoInterpreter {
    rec: InterpShared,
}

impl EchoInterpreter {
    fn with_items(items: Vec<SurfaceItem>) -> (Self, InterpShared) {
        let rec: InterpShared = Rc::new(RefCell::new(InterpRecorder {
            pending: items,
            ..InterpRecorder::default()
        }));
        (Self { rec: rec.clone() }, rec)
    }
}

impl TerminalSurfacePort for EchoInterpreter {
    fn interpret(&mut self, bytes: &[u8]) -> Vec<SurfaceItem> {
        let mut rec = self.rec.borrow_mut();
        rec.interpreted.push(bytes.to_vec());
        std::mem::take(&mut rec.pending)
    }
    fn resize(&mut self, size: TerminalSize) {
        self.rec.borrow_mut().resized.push(size);
    }
    fn active_row(&self) -> Option<Row> {
        self.rec.borrow().active.clone()
    }
    fn flush(&mut self) -> Vec<SurfaceItem> {
        std::mem::take(&mut self.rec.borrow_mut().pending)
    }
    fn modes(&self) -> TerminalModes {
        self.rec.borrow().modes
    }
}

fn row(text: &str) -> Row {
    Row {
        spans: vec![Span::new(TextStyle::default(), text.to_string())],
    }
}

fn event(event: SemanticEvent) -> SurfaceItem {
    SurfaceItem::Semantic(event)
}

fn lifecycle_items() -> Vec<SurfaceItem> {
    vec![
        event(SemanticEvent::DirectoryChanged {
            path: PathBuf::from("/tmp"),
        }),
        event(SemanticEvent::PromptStart),
        SurfaceItem::Row(row("user@host:~$ ")),
        event(SemanticEvent::PromptEnd),
        event(SemanticEvent::CommandStart),
        event(SemanticEvent::CommandLine {
            text: "echo hi".into(),
        }),
        SurfaceItem::Row(row("hi")),
        event(SemanticEvent::CommandFinished { exit_code: 0 }),
    ]
}

fn completed_command_items(command: &str) -> Vec<SurfaceItem> {
    vec![
        event(SemanticEvent::PromptStart),
        event(SemanticEvent::CommandStart),
        event(SemanticEvent::CommandLine {
            text: command.into(),
        }),
        event(SemanticEvent::CommandFinished { exit_code: 0 }),
    ]
}

fn service_with(items: Vec<SurfaceItem>) -> (SessionService, InterpShared, PtyShared) {
    let (interp, interp_rec) = EchoInterpreter::with_items(items);
    let (mock, pty_rec) = MockPty::new();
    let service = SessionService::new(Box::new(mock), Box::new(interp));
    (service, interp_rec, pty_rec)
}

fn feed(service: &mut SessionService, interp_rec: &InterpShared, items: Vec<SurfaceItem>) {
    interp_rec.borrow_mut().pending = items;
    service.ingest_bytes(b"chunk");
}

fn open_prompt(service: &mut SessionService, interp_rec: &InterpShared) {
    feed(service, interp_rec, vec![event(SemanticEvent::PromptStart)]);
}

fn insert(service: &mut SessionService, text: &str) {
    service.edit(EditCommand::Insert(text.into()));
}

fn chord(token: KeyToken) -> KeyChord {
    KeyChord {
        token,
        shift: false,
        alt: false,
        control: false,
    }
}

#[test]
fn ingest_delivers_an_ordered_lifecycle_to_one_completed_block() {
    let (mut service, interp_rec, _pty_rec) = service_with(lifecycle_items());

    service.ingest_bytes(b"chunk");

    assert_eq!(interp_rec.borrow().interpreted, [b"chunk".to_vec()]);
    assert_eq!(service.blocks().len(), 1);
    let block = &service.blocks()[0];
    assert_eq!(block.command(), Some("echo hi"));
    assert_eq!(block.cwd(), Some(&PathBuf::from("/tmp")));
    assert_eq!(block.status(), BlockStatus::Succeeded);
    assert!(block.finished().is_some());
    assert_eq!(block.copy(CopyKind::Output), "hi");
    assert_eq!(block.copy(CopyKind::WholeBlock), "echo hi\nhi");
    assert_eq!(
        service.phase(),
        ShellPhase::Prompting { prompt_open: false }
    );
}

#[test]
fn editor_state_follows_the_block_lifecycle() {
    let (mut service, interp_rec, _pty_rec) = service_with(Vec::new());

    assert_eq!(service.editor_state(), EditorState::DisabledCold);

    open_prompt(&mut service, &interp_rec);
    assert_eq!(service.editor_state(), EditorState::Idle);

    insert(&mut service, "echo hi");
    assert_eq!(service.submit(), SubmitOutcome::Submitted);
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);

    feed(
        &mut service,
        &interp_rec,
        vec![event(SemanticEvent::CommandStart)],
    );
    assert_eq!(service.editor_state(), EditorState::Running);

    feed(
        &mut service,
        &interp_rec,
        vec![event(SemanticEvent::CommandFinished { exit_code: 0 })],
    );
    // A submission is outstanding until the next `A` unlocks the editor.
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);

    open_prompt(&mut service, &interp_rec);
    assert_eq!(service.editor_state(), EditorState::Idle);

    service.finalize_stream();
    assert_eq!(service.editor_state(), EditorState::DisabledClosed);
}

#[test]
fn local_editing_is_accepted_only_at_idle() {
    let (mut service, interp_rec, _pty_rec) = service_with(Vec::new());

    insert(&mut service, "cold");
    assert_eq!(service.editor().text(), "");

    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "ab");
    assert_eq!(service.editor().text(), "ab");

    service.submit();
    insert(&mut service, "locked");
    assert_eq!(service.editor().text(), "");
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);
}

#[test]
fn edit_commands_drive_the_editor_api() {
    let (mut service, _interp_rec, _pty_rec) =
        service_with(vec![event(SemanticEvent::PromptStart)]);
    service.ingest_bytes(b"chunk");

    insert(&mut service, "abc");
    service.edit(EditCommand::MoveLeft { extend: false });
    service.edit(EditCommand::Backspace);
    assert_eq!(service.editor().text(), "ac");
    service.edit(EditCommand::Delete);
    assert_eq!(service.editor().text(), "a");
    service.edit(EditCommand::MoveRight { extend: false });
    service.edit(EditCommand::Home { extend: false });
    assert_eq!(service.editor().caret(), 0);
    service.edit(EditCommand::End { extend: false });
    assert_eq!(service.editor().caret(), 1);

    service.edit(EditCommand::PageUp);
    service.edit(EditCommand::PageDown);
    service.edit(EditCommand::MoveToCell { line: 0, cell: 0 });
    assert_eq!(service.editor().caret(), 0);
    service.edit(EditCommand::DragToCell { line: 0, cell: 1 });
    assert_eq!(service.editor().selected_text(), Some("a"));

    service.edit(EditCommand::SelectAll);
    assert_eq!(service.editor().selected_text(), Some("a"));
    service.edit(EditCommand::Paste("z".into()));
    assert_eq!(service.editor().text(), "z");
    service.edit(EditCommand::Clear);
    assert_eq!(service.editor().text(), "");
}

#[test]
fn submit_writes_one_call_with_a_normalized_terminator() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);

    insert(&mut service, "echo hi\n");
    assert_eq!(service.submit(), SubmitOutcome::Submitted);

    assert_eq!(pty_rec.borrow().writes, [b"echo hi\r".to_vec()]);
    assert_eq!(service.editor().text(), "");
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);

    feed(
        &mut service,
        &interp_rec,
        vec![event(SemanticEvent::CommandStart)],
    );
    assert_eq!(service.blocks()[0].command(), Some("echo hi"));
}

#[test]
fn submit_strips_exactly_one_trailing_newline() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);

    insert(&mut service, "a\n\n");
    assert_eq!(service.submit(), SubmitOutcome::Submitted);

    assert_eq!(pty_rec.borrow().writes, [b"a\n\r".to_vec()]);
}

#[test]
fn whitespace_only_submissions_are_ignored_and_keep_the_text() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);

    insert(&mut service, "  \n\t");
    assert_eq!(service.submit(), SubmitOutcome::Ignored);

    assert!(pty_rec.borrow().writes.is_empty());
    assert_eq!(service.editor().text(), "  \n\t");
    assert_eq!(service.editor_state(), EditorState::Idle);
}

#[test]
fn a_failed_write_keeps_the_text_and_leaves_the_editor_idle() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "ls");
    pty_rec.borrow_mut().fail_writes = true;

    assert_eq!(service.submit(), SubmitOutcome::Failed);

    assert_eq!(service.editor().text(), "ls");
    assert_eq!(service.editor_state(), EditorState::Idle);
    assert!(pty_rec.borrow().writes.is_empty());

    pty_rec.borrow_mut().fail_writes = false;
    assert_eq!(service.submit(), SubmitOutcome::Submitted);
    assert_eq!(pty_rec.borrow().writes, [b"ls\r".to_vec()]);
}

#[test]
fn ctrl_c_at_idle_clears_the_editor_and_signals() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "draft");

    service.interrupt();

    assert_eq!(service.editor().text(), "");
    assert_eq!(service.editor_state(), EditorState::Idle);
    assert_eq!(pty_rec.borrow().writes, [vec![0x03]]);
}

#[test]
fn ctrl_c_while_awaiting_restores_the_submission() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "if true");
    service.submit();

    service.interrupt();

    assert_eq!(service.editor().text(), "if true");
    assert_eq!(service.editor_state(), EditorState::Idle);
    assert_eq!(pty_rec.borrow().writes, [b"if true\r".to_vec(), vec![0x03]]);
}

#[test]
fn ctrl_c_while_running_only_signals() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "sleep 1");
    service.submit();
    feed(
        &mut service,
        &interp_rec,
        vec![event(SemanticEvent::CommandStart)],
    );

    service.interrupt();

    assert_eq!(service.editor().text(), "");
    assert_eq!(service.editor_state(), EditorState::Running);
    assert_eq!(pty_rec.borrow().writes, [b"sleep 1\r".to_vec(), vec![0x03]]);
}

#[test]
fn finished_without_start_restores_the_pending_text() {
    let (mut service, interp_rec, _pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "bad syntax");
    service.submit();

    feed(
        &mut service,
        &interp_rec,
        vec![event(SemanticEvent::CommandFinished { exit_code: 1 })],
    );

    assert_eq!(service.editor().text(), "bad syntax");
    assert_eq!(service.editor_state(), EditorState::AwaitingStart);

    open_prompt(&mut service, &interp_rec);
    assert_eq!(service.editor_state(), EditorState::Idle);
    assert_eq!(service.editor().text(), "bad syntax");
}

#[test]
fn closing_the_session_refills_the_editor_with_pending_text() {
    let (mut service, interp_rec, _pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);
    insert(&mut service, "pending command");
    service.submit();

    service.finalize_stream();

    assert_eq!(service.editor().text(), "pending command");
    assert_eq!(service.editor_state(), EditorState::DisabledClosed);
}

#[test]
fn history_recalls_newest_first_and_resets_on_edit() {
    let mut items = Vec::new();
    for command in ["one", "one", "two"] {
        items.extend(completed_command_items(command));
    }
    items.push(event(SemanticEvent::PromptStart));
    let (mut service, _interp_rec, _pty_rec) = service_with(items);
    service.ingest_bytes(b"chunk");

    service.edit(EditCommand::MoveUp { extend: false });
    assert_eq!(service.editor().text(), "two");
    service.edit(EditCommand::MoveUp { extend: false });
    assert_eq!(service.editor().text(), "one");
    service.edit(EditCommand::MoveUp { extend: false });
    assert_eq!(service.editor().text(), "one");

    service.edit(EditCommand::MoveDown { extend: false });
    assert_eq!(service.editor().text(), "two");
    service.edit(EditCommand::MoveDown { extend: false });
    assert_eq!(service.editor().text(), "");

    service.history_up();
    insert(&mut service, "!");
    assert_eq!(service.editor().text(), "two!");
    service.history_up();
    assert_eq!(service.editor().text(), "two");
}

#[test]
fn locked_paste_is_sanitized_and_written_to_the_pty() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());
    open_prompt(&mut service, &interp_rec);

    service.paste_to_pty("ignored").unwrap();
    assert!(pty_rec.borrow().writes.is_empty());

    insert(&mut service, "cat");
    service.submit();
    feed(
        &mut service,
        &interp_rec,
        vec![event(SemanticEvent::CommandStart)],
    );

    service.paste_to_pty("a\r\nb\tc\u{1}").unwrap();

    assert_eq!(
        pty_rec.borrow().writes.last().map(Vec::as_slice),
        Some(b"a\nb\tc".as_slice())
    );
}

#[test]
fn encode_key_reads_the_surface_modes() {
    let (service, interp_rec, _pty_rec) = service_with(Vec::new());
    let up = chord(KeyToken::Named(NamedKey::Up));

    assert_eq!(service.encode_key(&up), Some(b"\x1b[A".to_vec()));

    interp_rec.borrow_mut().modes.set_app_cursor(true);
    assert_eq!(service.encode_key(&up), Some(b"\x1bOA".to_vec()));
}

#[test]
fn editor_geometry_drives_wrapping_and_clamps_the_visible_lines() {
    let (mut service, _interp_rec, _pty_rec) =
        service_with(vec![event(SemanticEvent::PromptStart)]);
    service.ingest_bytes(b"chunk");

    service.set_editor_geometry(5, 99);
    assert_eq!(service.editor_visual_lines(), 28);
    service.set_editor_geometry(0, 0);
    assert_eq!(service.editor_visual_lines(), 1);

    service.set_editor_geometry(5, 2);
    assert_eq!(service.editor_visual_lines(), 2);
    insert(&mut service, "abcdefgh");
    assert_eq!(service.editor().visual_line_count(), 2);

    let view = service.editor_view();
    assert_eq!(view.lines.len(), 2);
    assert_eq!(view.lines[0].text, "abcde");
    assert_eq!(view.lines[1].text, "fgh");
    assert_eq!(view.caret, (1, 3));
}

#[test]
fn finalize_flushes_the_partial_row_and_interrupts_the_running_block() {
    let (mut service, interp_rec, _pty_rec) = service_with(vec![
        event(SemanticEvent::PromptStart),
        event(SemanticEvent::CommandStart),
        SurfaceItem::Row(row("before")),
    ]);
    service.ingest_bytes(b"chunk");
    interp_rec
        .borrow_mut()
        .pending
        .push(SurfaceItem::Row(row("tail-of-stream")));

    service.finalize_stream();

    let block = &service.blocks()[0];
    assert_eq!(block.status(), BlockStatus::Interrupted);
    assert_eq!(block.copy(CopyKind::Output), "before\ntail-of-stream");
    assert_eq!(service.phase(), ShellPhase::Closed);
}

#[test]
fn terminal_reset_interrupts_the_block_and_later_items_are_dropped() {
    let (mut service, interp_rec, _pty_rec) = service_with(vec![
        event(SemanticEvent::PromptStart),
        event(SemanticEvent::CommandStart),
        SurfaceItem::Row(row("partial")),
        event(SemanticEvent::TerminalReset),
    ]);
    service.ingest_bytes(b"chunk");
    interp_rec
        .borrow_mut()
        .pending
        .push(SurfaceItem::Row(row("after-reset")));

    service.ingest_bytes(b"late");

    let block = &service.blocks()[0];
    assert_eq!(block.status(), BlockStatus::Interrupted);
    assert_eq!(block.copy(CopyKind::Output), "partial");
    assert_eq!(service.phase(), ShellPhase::Closed);
}

#[test]
fn live_row_is_gated_by_the_phase() {
    let (mut service, interp_rec, _pty_rec) = service_with(vec![
        event(SemanticEvent::PromptStart),
        event(SemanticEvent::CommandStart),
    ]);
    interp_rec.borrow_mut().active = Some(row("prompt live"));

    assert_eq!(
        service.live_row().map(|row| row.spans[0].text.clone()),
        None
    );

    service.ingest_bytes(b"open");
    assert_eq!(
        service.live_row().map(|row| row.spans[0].text.clone()),
        Some("prompt live".into())
    );
}

#[test]
fn copy_block_routes_to_the_owning_block() {
    let (mut service, _interp_rec, _pty_rec) = service_with(lifecycle_items());
    service.ingest_bytes(b"chunk");
    let id = service.blocks()[0].id();

    assert_eq!(
        service.copy_block(id, CopyKind::Command),
        Some("echo hi".into())
    );
    assert_eq!(service.copy_block(id, CopyKind::Output), Some("hi".into()));
    assert_eq!(
        service.copy_block(id, CopyKind::WholeBlock),
        Some("echo hi\nhi".into())
    );
    assert_eq!(service.copy_block(BlockId::new(99), CopyKind::Output), None);
}

#[test]
fn keystrokes_and_resize_route_through_the_ports() {
    let (mut service, interp_rec, pty_rec) = service_with(Vec::new());

    service.write_keystrokes(b"ls\r").unwrap();
    service.resize(TerminalSize::new(100, 40)).unwrap();

    let rec = pty_rec.borrow();
    assert_eq!(rec.writes, [b"ls\r".to_vec()]);
    assert!(rec.sizes.contains(&TerminalSize::new(100, 40)));
    assert_eq!(interp_rec.borrow().resized, [TerminalSize::new(100, 40)]);
}
