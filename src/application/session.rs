//! SessionService + SessionDocument — Phase 1 orchestration
//! (`knowledge/specs/pty-raw-output/{spec,design}.md`; design § 2, § 3).

use std::collections::VecDeque;

use crate::domain::interpreter::TextInterpreterPort;
use crate::domain::output::Row;
use crate::domain::pty::{PtyPort, PtySize};

/// Session-document line cap (design § 4; spec BR-4: bounded memory,
/// drop-oldest with the newest always retained). In Phase 2 the same
/// policy moves to per-block scope when blocks appear.
pub const OUTPUT_MAX_ROWS: usize = 4_000;

/// The bounded styled-row log of the current session.
///
/// Bounded by construction (architecture Common Mistakes #5): when the
/// cap is reached the oldest completed rows are evicted. Rows keep the
/// wrap width they were created with — resize never re-flows them
/// (spec BR-7).
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
    /// Creates the document with an explicit cap (tests shrink it; the
    /// production default is `OUTPUT_MAX_ROWS`).
    pub fn with_cap(cap: usize) -> Self {
        Self {
            rows: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    /// Push one completed row, evicting the oldest while above the cap.
    pub fn push_row(&mut self, row: Row) {
        while self.rows.len() >= self.cap {
            self.rows.pop_front();
        }
        self.rows.push_back(row);
    }

    /// Push a batch of completed rows (one writer chunk's worth). A batch
    /// larger than the cap keeps only its newest tail (the newest content
    /// is always retained, spec BR-4).
    pub fn push_rows(&mut self, rows: impl IntoIterator<Item = Row>) {
        for row in rows {
            self.push_row(row);
        }
    }

    /// Read-only view for rendering (Presentation never mutates here).
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

/// Orchestrates the single Phase 1 session: shell spawn, keystroke
/// passthrough routing, byte ingestion, and resize — without ever
/// touching the PTY, the interpreter implementation, or gpui.
pub struct SessionService {
    pty: Box<dyn PtyPort>,
    interpreter: Box<dyn TextInterpreterPort>,
    document: SessionDocument,
}

impl SessionService {
    pub fn new(pty: Box<dyn PtyPort>, interpreter: Box<dyn TextInterpreterPort>) -> Self {
        Self {
            pty,
            interpreter,
            document: SessionDocument::default(),
        }
    }

    /// Spawn the shell at the given size (BR-1: single session;
    /// `PtyPort::spawn` is idempotent while a live session exists).
    #[allow(dead_code)]
    pub fn spawn_session(&mut self, size: PtySize) -> crate::common::errors::Result<()> {
        self.pty.spawn(size)
    }

    /// Application-side ingest (design § 3): interpret a raw chunk and
    /// append its rows to the bounded document. The raw bytes are all
    /// consumed — only interpreted rows can be evicted later by the cap
    /// (BR-3/BR-4).
    pub fn ingest_bytes(&mut self, bytes: &[u8]) {
        let rows = self.interpreter.interpret(bytes);
        self.document.push_rows(rows);
    }

    /// EOF finalization (BR-9): flush the interpreter's last partial row
    /// into the document. Idempotent; safe to call after `is_alive`
    /// turned false.
    pub fn finalize_stream(&mut self) {
        let rows = self.interpreter.flush();
        self.document.push_rows(rows);
    }

    /// Keystroke passthrough (BR-8): raw bytes to the PTY, routed through
    /// this service — Presentation never writes the PTY directly.
    pub fn write_keystrokes(&mut self, bytes: &[u8]) -> crate::common::errors::Result<()> {
        self.pty.write(bytes)
    }

    /// Resize path (BR-7): immediately forward to the PTY (kernel
    /// SIGWINCH) and shift the interpreter's wrap width for incoming
    /// bytes. No debounce belongs here; stored rows are untouched.
    pub fn resize(&mut self, size: PtySize) -> crate::common::errors::Result<()> {
        self.interpreter.resize_cols(size.cols);
        self.pty.resize(size)
    }

    /// Bounded document read access for Presentation.
    pub fn document(&self) -> &SessionDocument {
        &self.document
    }

    /// Session aliveness (BR-9): false once EOF/death was observed.
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

    // -- Test doubles (headless: no real PTY, no interpreter) ----------

    /// Recorded `PtyPort` calls, shared with the test via `Rc` (the live
    /// double lives inside a `Box` owned by the service).
    #[derive(Default, Debug)]
    struct PtyRecorder {
        spawn_calls: usize,
        sizes: Vec<PtySize>,
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
        fn spawn(&mut self, size: PtySize) -> crate::common::errors::Result<()> {
            let mut rec = self.rec.borrow_mut();
            // BR-1: a second spawn while the session is live is a no-op.
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
        fn resize(&mut self, size: PtySize) -> crate::common::errors::Result<()> {
            self.rec.borrow_mut().sizes.push(size);
            Ok(())
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
    }

    /// Recorded interpreter calls: pending rows are drained by
    /// `interpret`/`flush`; `resize_cols` appends.
    #[derive(Default, Debug)]
    struct InterpRecorder {
        pending: Vec<Row>,
        resized: Vec<u16>,
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

    impl TextInterpreterPort for EchoInterpreter {
        fn interpret(&mut self, _bytes: &[u8]) -> Vec<Row> {
            std::mem::take(&mut self.rec.borrow_mut().pending)
        }
        fn resize_cols(&mut self, cols: u16) {
            self.rec.borrow_mut().resized.push(cols);
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

    // -- SessionDocument (golden family 7: line-cap eviction order) ----

    #[test]
    fn line_cap_evicts_oldest_completed_rows_and_keeps_newest() {
        let mut doc = SessionDocument::with_cap(3);
        doc.push_rows([row("a"), row("b"), row("c"), row("d"), row("e")]);
        // Drop-oldest; the newest content is always retained (BR-4).
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

    // -- SessionService orchestration -----------------------------------

    #[test]
    fn spawn_is_idempotent_while_session_is_alive() {
        let (mock, rec) = MockPty::new(true);
        let mut service = SessionService::new(
            Box::new(mock),
            Box::new(EchoInterpreter {
                rec: Rc::new(RefCell::new(InterpRecorder::default())),
            }),
        );
        service.spawn_session(PtySize::new(80, 24)).unwrap();
        service.spawn_session(PtySize::new(80, 24)).unwrap();
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
        service.resize(PtySize::new(100, 40)).unwrap();

        let rec = pty_rec.borrow();
        assert_eq!(rec.writes, [b"ls\r".to_vec()]);
        assert!(rec.sizes.contains(&PtySize::new(100, 40)));
        assert_eq!(interp_rec.borrow().resized, [100]);
    }
}
