//! Session service unit tests: routing through mock PTY and surface ports.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::SessionService;
use crate::domain::block::{BlockId, BlockStatus, CopyKind};
use crate::domain::output::{Row, Span, TextStyle};
use crate::domain::pty::PtyPort;
use crate::domain::rows::BoundedRows;
use crate::domain::semantic::{SemanticEvent, ShellPhase};
use crate::domain::size::TerminalSize;
use crate::domain::terminal_surface::{SurfaceItem, TerminalSurfacePort};

/// Recorded `PtyPort` calls shared with the test through `Rc`.
#[derive(Default, Debug)]
struct PtyRecorder {
    sizes: Vec<TerminalSize>,
    writes: Vec<Vec<u8>>,
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
        self.rec.borrow_mut().writes.push(bytes.to_vec());
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
}

fn row(text: &str) -> Row {
    Row {
        spans: vec![Span::new(TextStyle::default(), text.to_string())],
    }
}

fn event(event: SemanticEvent) -> SurfaceItem {
    SurfaceItem::Semantic(event)
}

fn texts(rows: &BoundedRows) -> Vec<String> {
    rows.iter()
        .map(|row| row.spans.iter().map(|span| span.text.as_str()).collect())
        .collect()
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

fn service_with(items: Vec<SurfaceItem>) -> (SessionService, InterpShared, PtyShared) {
    let (interp, interp_rec) = EchoInterpreter::with_items(items);
    let (mock, pty_rec) = MockPty::new();
    let service = SessionService::new(Box::new(mock), Box::new(interp));
    (service, interp_rec, pty_rec)
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
fn prompt_rows_clear_when_the_command_starts() {
    let (mut service, _interp_rec, _pty_rec) = service_with(vec![
        event(SemanticEvent::PromptStart),
        SurfaceItem::Row(row("prompt$ ")),
        event(SemanticEvent::CommandStart),
    ]);

    service.ingest_bytes(b"chunk");

    assert!(service.prompt_rows().is_empty());
    assert_eq!(service.blocks().len(), 1);
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
fn prompt_rows_are_bounded_and_visible_before_the_command_starts() {
    let (mut service, _interp_rec, _pty_rec) = service_with(vec![
        event(SemanticEvent::PromptStart),
        SurfaceItem::Row(row("prompt$ ")),
    ]);
    service.ingest_bytes(b"chunk");

    assert_eq!(texts(service.prompt_rows()), ["prompt$ "]);
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
