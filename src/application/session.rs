//! SessionService + SessionDocument — session orchestration.

use std::collections::VecDeque;

use crate::domain::output::Row;
use crate::domain::pty::PtyPort;
use crate::domain::size::TerminalSize;
use crate::domain::terminal_surface::TerminalSurfacePort;

/// Session-document line cap. Bounded memory: the oldest completed rows are
/// evicted, the newest content always survives.
pub const OUTPUT_MAX_ROWS: usize = 4_000;

/// The bounded styled-row log of the current session.
///
/// Rows keep the wrap width they were created with; resize never re-flows
/// them.
#[derive(Debug)]
pub struct SessionDocument {
    rows: VecDeque<Row>,
    cap: usize,
}

impl Default for SessionDocument {
    fn default() -> Self {
        Self::with_cap(OUTPUT_MAX_ROWS)
    }
}

impl SessionDocument {
    /// Creates the document with an explicit cap (tests shrink it).
    pub fn with_cap(cap: usize) -> Self {
        Self {
            rows: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    pub fn push_row(&mut self, row: Row) {
        while self.rows.len() >= self.cap {
            self.rows.pop_front();
        }
        self.rows.push_back(row);
    }

    /// Pushes a batch of completed rows. A batch larger than the cap keeps
    /// only its newest tail.
    pub fn push_rows(&mut self, rows: impl IntoIterator<Item = Row>) {
        for row in rows {
            self.push_row(row);
        }
    }

    pub fn rows(&self) -> &VecDeque<Row> {
        &self.rows
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Orchestrates one session: shell spawn, keystroke routing, byte ingestion,
/// and resize. It never touches the PTY implementation, the interpreter
/// implementation, or gpui.
pub struct SessionService {
    pty: Box<dyn PtyPort>,
    surface: Box<dyn TerminalSurfacePort>,
    document: SessionDocument,
}

impl SessionService {
    pub fn new(pty: Box<dyn PtyPort>, surface: Box<dyn TerminalSurfacePort>) -> Self {
        Self {
            pty,
            surface,
            document: SessionDocument::default(),
        }
    }

    /// Idempotent while a live session exists.
    #[allow(dead_code)]
    pub fn spawn_session(&mut self, size: TerminalSize) -> crate::common::errors::Result<()> {
        self.pty.spawn(size)
    }

    /// Interprets a raw chunk and appends its rows to the bounded document.
    /// All raw bytes are consumed; only interpreted rows can be evicted by
    /// the cap.
    pub fn ingest_bytes(&mut self, bytes: &[u8]) {
        let rows = self.surface.interpret(bytes);
        self.document.push_rows(rows);
    }

    /// Flushes the interpreter's last partial row into the document.
    /// Idempotent; safe to call after `is_alive` turned false.
    pub fn finalize_stream(&mut self) {
        let rows = self.surface.flush();
        self.document.push_rows(rows);
    }

    /// Raw bytes to the PTY. Presentation never writes the PTY directly.
    pub fn write_keystrokes(&mut self, bytes: &[u8]) -> crate::common::errors::Result<()> {
        self.pty.write(bytes)
    }

    /// Immediately forwards to the PTY (SIGWINCH) and shifts the surface's
    /// wrap width for incoming bytes. No debounce; stored rows are untouched.
    pub fn resize(&mut self, size: TerminalSize) -> crate::common::errors::Result<()> {
        self.surface.resize(size);
        self.pty.resize(size)
    }

    pub fn document(&self) -> &SessionDocument {
        &self.document
    }

    /// False once EOF or child death was observed.
    #[allow(dead_code)]
    pub fn is_alive(&self) -> bool {
        self.pty.is_alive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::domain::output::{Span, TextStyle};

    /// Recorded `PtyPort` calls shared with the test through `Rc`.
    #[derive(Default, Debug)]
    struct PtyRecorder {
        spawn_calls: usize,
        sizes: Vec<TerminalSize>,
        writes: Vec<Vec<u8>>,
    }

    type PtyShared = Rc<RefCell<PtyRecorder>>;

    struct MockPty {
        rec: PtyShared,
        alive: bool,
    }

    impl MockPty {
        fn new(alive: bool) -> (Self, PtyShared) {
            let rec: PtyShared = Rc::new(RefCell::new(PtyRecorder::default()));
            (
                Self {
                    rec: rec.clone(),
                    alive,
                },
                rec,
            )
        }
    }

    impl PtyPort for MockPty {
        fn spawn(&mut self, size: TerminalSize) -> crate::common::errors::Result<()> {
            let mut rec = self.rec.borrow_mut();
            if rec.spawn_calls > 0 && self.alive {
                return Ok(());
            }
            rec.spawn_calls += 1;
            rec.sizes.push(size);
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
            self.alive
        }
    }

    #[derive(Default, Debug)]
    struct InterpRecorder {
        pending: Vec<Row>,
        resized: Vec<TerminalSize>,
    }

    type InterpShared = Rc<RefCell<InterpRecorder>>;

    struct EchoInterpreter {
        rec: InterpShared,
    }

    impl EchoInterpreter {
        fn with_pending(texts: &[&str]) -> (Self, InterpShared) {
            let rec: InterpShared = Rc::new(RefCell::new(InterpRecorder {
                pending: texts.iter().map(|t| row(t)).collect(),
                resized: Vec::new(),
            }));
            (Self { rec: rec.clone() }, rec)
        }
    }

    impl TerminalSurfacePort for EchoInterpreter {
        fn interpret(&mut self, _bytes: &[u8]) -> Vec<Row> {
            std::mem::take(&mut self.rec.borrow_mut().pending)
        }
        fn resize(&mut self, size: TerminalSize) {
            self.rec.borrow_mut().resized.push(size);
        }
        fn flush(&mut self) -> Vec<Row> {
            std::mem::take(&mut self.rec.borrow_mut().pending)
        }
    }

    fn row(text: &str) -> Row {
        Row {
            spans: vec![Span::new(TextStyle::default(), text.to_string())],
        }
    }

    fn texts(container: &SessionDocument) -> Vec<String> {
        container
            .rows()
            .iter()
            .map(|r| r.spans.iter().map(|span| span.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn line_cap_evicts_oldest_completed_rows_and_keeps_newest() {
        let mut doc = SessionDocument::with_cap(3);
        doc.push_rows([row("a"), row("b"), row("c"), row("d"), row("e")]);
        assert_eq!(texts(&doc), ["c", "d", "e"]);
        assert_eq!(doc.len(), 3);
    }

    #[test]
    fn line_cap_batch_bigger_than_cap_keeps_newest_tail() {
        let mut doc = SessionDocument::with_cap(4);
        doc.push_rows((1..=9).map(|n| row(&n.to_string())));
        assert_eq!(texts(&doc), ["6", "7", "8", "9"]);
    }

    #[test]
    fn default_cap_is_output_max_rows() {
        assert_eq!(OUTPUT_MAX_ROWS, 4_000);
        assert!(SessionDocument::default().is_empty());
    }

    #[test]
    fn spawn_is_idempotent_while_session_is_alive() {
        let (mock, rec) = MockPty::new(true);
        let mut service = SessionService::new(
            Box::new(mock),
            Box::new(EchoInterpreter {
                rec: Rc::new(RefCell::new(InterpRecorder::default())),
            }),
        );
        service.spawn_session(TerminalSize::new(80, 24)).unwrap();
        service.spawn_session(TerminalSize::new(80, 24)).unwrap();
        assert_eq!(rec.borrow().spawn_calls, 1);
    }

    #[test]
    fn ingest_delivers_interpreted_rows_to_the_bounded_document() {
        let (interp, _rec) = EchoInterpreter::with_pending(&["ping", "pong"]);
        let (mock, _pty_rec) = MockPty::new(true);
        let mut service = SessionService::new(Box::new(mock), Box::new(interp));
        service.ingest_bytes(b"chunk");
        assert_eq!(texts(service.document()), ["ping", "pong"]);
    }

    #[test]
    fn finalize_flushes_interpreter_remainder() {
        let (interp, _rec) = EchoInterpreter::with_pending(&["tail-of-stream"]);
        let (mock, _pty_rec) = MockPty::new(false);
        let mut service = SessionService::new(Box::new(mock), Box::new(interp));
        service.finalize_stream();
        assert!(!service.document().is_empty());
    }

    #[test]
    fn keystrokes_and_resize_route_through_the_ports() {
        let (mock, pty_rec) = MockPty::new(true);
        let (interp, interp_rec) = EchoInterpreter::with_pending(&[]);
        let mut service = SessionService::new(Box::new(mock), Box::new(interp));

        service.write_keystrokes(b"ls\r").unwrap();
        service.resize(TerminalSize::new(100, 40)).unwrap();

        let rec = pty_rec.borrow();
        assert_eq!(rec.writes, [b"ls\r".to_vec()]);
        assert!(rec.sizes.contains(&TerminalSize::new(100, 40)));
        assert_eq!(interp_rec.borrow().resized, [TerminalSize::new(100, 40)]);
    }
}
