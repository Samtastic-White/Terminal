//! Session orchestration: editor lifecycle, keystroke routing, byte ingestion,
//! and resize. It never touches the PTY implementation, the interpreter
//! implementation, or gpui.

// The presentation dock consumes this editor API in T6; until that wiring
// exists the unit tests are the only users in a non-test build.
#![cfg_attr(not(test), allow(dead_code))]

use tracing::{debug, trace, warn};

use crate::application::block::{BlockService, Timestamp};
use crate::common::errors::Result;
use crate::domain::block::{Block, BlockId, CopyKind};
use crate::domain::editor::{
    Direction, Editor, EditorLimits, EditorView, MAX_VISIBLE_LINES, sanitize_paste,
};
use crate::domain::keymap::{KeyChord, encode_key};
use crate::domain::output::Row;
use crate::domain::pty::PtyPort;
use crate::domain::rows::BoundedRows;
use crate::domain::semantic::ShellPhase;
use crate::domain::size::TerminalSize;
use crate::domain::terminal_surface::{SurfaceItem, TerminalSurfacePort};

/// Observable editor state derived from the block lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorState {
    DisabledCold,
    Idle,
    AwaitingStart,
    Running,
    DisabledClosed,
}

/// Local editing commands available while the editor is idle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditCommand {
    Insert(String),
    Paste(String),
    Backspace,
    Delete,
    MoveLeft { extend: bool },
    MoveRight { extend: bool },
    MoveUp { extend: bool },
    MoveDown { extend: bool },
    Home { extend: bool },
    End { extend: bool },
    PageUp,
    PageDown,
    MoveToCell { line: usize, cell: usize },
    DragToCell { line: usize, cell: usize },
    SelectAll,
    Clear,
}

/// Result of an `Enter` on the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Empty/whitespace-only text or a locked editor: nothing was written.
    Ignored,
    /// The command was written in one call and the editor is now locked.
    Submitted,
    /// The PTY write failed; the editor kept its text.
    Failed,
}

/// Orchestrates one session: editor lifecycle, byte ingestion, and resize.
pub struct SessionService {
    pty: Box<dyn PtyPort>,
    surface: Box<dyn TerminalSurfacePort>,
    blocks: BlockService,
    editor: Editor,
    history_cursor: Option<usize>,
    editor_cols: usize,
    editor_visible_lines: usize,
}

impl SessionService {
    pub fn new(pty: Box<dyn PtyPort>, surface: Box<dyn TerminalSurfacePort>) -> Self {
        let mut editor = Editor::new(EditorLimits::default());
        editor.set_wrap_cols(80);
        Self {
            pty,
            surface,
            blocks: BlockService::default(),
            editor,
            history_cursor: None,
            editor_cols: 80,
            editor_visible_lines: 1,
        }
    }

    /// Interprets a raw chunk and routes its rows and semantic markers through
    /// the block state machine. All raw bytes are consumed; only interpreted
    /// rows can be evicted by the bounded block and session budgets.
    pub fn ingest_bytes(&mut self, bytes: &[u8]) {
        let items = self.surface.interpret(bytes);
        let rows = items
            .iter()
            .filter(|item| matches!(item, SurfaceItem::Row(_)))
            .count();
        trace!(bytes = bytes.len(), rows, "session ingest");
        self.blocks.handle_items(items, Timestamp::now());
        self.reconcile();
    }

    /// Flushes the interpreter's last partial row and then finalizes the
    /// session: a still-running block becomes Interrupted. Idempotent; safe to
    /// call after the reader channel disconnected.
    pub fn finalize_stream(&mut self) {
        let items = self.surface.flush();
        self.blocks.handle_items(items, Timestamp::now());
        self.blocks.session_ended(Timestamp::now());
        self.reconcile();
    }

    /// Blocks in stream order, oldest first.
    pub fn blocks(&self) -> &[Block] {
        self.blocks.blocks()
    }

    /// The surface row still being built, exposed only inside a running block.
    /// Committed rows never pass through here, so drawing the live row cannot
    /// duplicate a line.
    pub fn live_row(&self) -> Option<Row> {
        self.blocks.live_row(self.surface.active_row())
    }

    /// Rows received while a submission waits for `C` or `D`; rendered as a
    /// quiet overlay above the editor dock.
    pub fn feedback_rows(&self) -> &BoundedRows {
        self.blocks.feedback_rows()
    }

    /// Current shell position in the block lifecycle.
    pub fn phase(&self) -> ShellPhase {
        self.blocks.phase()
    }

    /// Plain-text extraction for one block; `None` if the id was evicted.
    pub fn copy_block(&self, id: BlockId, kind: CopyKind) -> Option<String> {
        self.blocks.copy_block(id, kind)
    }

    /// The command buffer, for tests and read-only view queries.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// State the key dispatcher and the dock render from.
    pub fn editor_state(&self) -> EditorState {
        if self.blocks.is_closed() {
            return EditorState::DisabledClosed;
        }
        match self.blocks.phase() {
            ShellPhase::Cold => EditorState::DisabledCold,
            ShellPhase::Running => EditorState::Running,
            ShellPhase::Prompting { .. } => {
                if self.blocks.awaiting_start() {
                    EditorState::AwaitingStart
                } else {
                    EditorState::Idle
                }
            }
            ShellPhase::Closed => EditorState::DisabledClosed,
        }
    }

    /// Visible editor window (at most 28 visual lines).
    pub fn editor_view(&self) -> EditorView {
        self.editor.view()
    }

    /// Height in visual lines, clamped to the dock's 1..=28 range.
    pub fn editor_visual_lines(&self) -> usize {
        self.editor_visible_lines.clamp(1, MAX_VISIBLE_LINES)
    }

    /// Called by the view before layout and PTY row derivation.
    pub fn set_editor_geometry(&mut self, cols: usize, visible_lines: usize) {
        let cols = cols.max(1);
        let visible_lines = visible_lines.clamp(1, MAX_VISIBLE_LINES);
        if self.editor_cols != cols {
            self.editor_cols = cols;
            self.editor.set_wrap_cols(cols);
        }
        if self.editor_visible_lines != visible_lines {
            self.editor_visible_lines = visible_lines;
            self.editor.set_visible_lines(visible_lines);
        }
    }

    /// Applies one local editing command; ignored outside `Idle`.
    pub fn edit(&mut self, command: EditCommand) {
        if self.editor_state() != EditorState::Idle {
            return;
        }
        let mutates = matches!(
            &command,
            EditCommand::Insert(_)
                | EditCommand::Paste(_)
                | EditCommand::Backspace
                | EditCommand::Delete
                | EditCommand::Clear
        );

        match command {
            EditCommand::Insert(text) => {
                self.editor.insert_text(&text);
            }
            EditCommand::Paste(text) => {
                let outcome = self.editor.insert_paste(&text);
                if outcome.dropped_bytes > 0 || outcome.dropped_lines > 0 {
                    debug!(
                        dropped_bytes = outcome.dropped_bytes,
                        dropped_lines = outcome.dropped_lines,
                        "editor paste truncated"
                    );
                }
            }
            EditCommand::Backspace => {
                self.editor.delete_backward();
            }
            EditCommand::Delete => {
                self.editor.delete_forward();
            }
            EditCommand::MoveLeft { extend } => {
                self.editor.move_horizontal(-1, extend);
            }
            EditCommand::MoveRight { extend } => {
                self.editor.move_horizontal(1, extend);
            }
            EditCommand::MoveUp { extend } => {
                let (line, _) = self.editor.caret_visual_position();
                if !extend && line == 0 {
                    self.history_up();
                } else {
                    self.editor.move_vertical(Direction::Up, extend);
                }
            }
            EditCommand::MoveDown { extend } => {
                let (line, _) = self.editor.caret_visual_position();
                let last = self.editor.visual_line_count().saturating_sub(1);
                if !extend && line >= last {
                    self.history_down();
                } else {
                    self.editor.move_vertical(Direction::Down, extend);
                }
            }
            EditCommand::Home { extend } => {
                self.editor.move_home(extend);
            }
            EditCommand::End { extend } => {
                self.editor.move_end(extend);
            }
            EditCommand::PageUp => {
                self.editor.move_page(Direction::Up, false);
            }
            EditCommand::PageDown => {
                self.editor.move_page(Direction::Down, false);
            }
            EditCommand::MoveToCell { line, cell } => {
                self.editor.move_to_cell(line, cell);
            }
            EditCommand::DragToCell { line, cell } => {
                self.editor.drag_to_cell(line, cell);
            }
            EditCommand::SelectAll => {
                self.editor.select_all();
            }
            EditCommand::Clear => {
                self.editor.clear();
            }
        }

        if mutates {
            self.blocks.clear_feedback();
            self.history_cursor = None;
        }
    }

    /// Writes the command in one call, then locks the editor as
    /// `AwaitingStart`. The trailing break is normalized to the single `CR`.
    pub fn submit(&mut self) -> SubmitOutcome {
        if self.editor_state() != EditorState::Idle {
            return SubmitOutcome::Ignored;
        }
        let text = self.editor.text();
        let submitted = text.strip_suffix('\n').unwrap_or(text).to_owned();
        if submitted.trim().is_empty() {
            return SubmitOutcome::Ignored;
        }

        let mut payload = Vec::with_capacity(submitted.len() + 1);
        payload.extend_from_slice(submitted.as_bytes());
        payload.push(b'\r');
        if let Err(error) = self.pty.write(&payload) {
            warn!(error = %error, "editor submit failed");
            return SubmitOutcome::Failed;
        }

        debug!(
            bytes = submitted.len(),
            lines = submitted.split('\n').count(),
            "editor submitted"
        );
        self.editor.clear();
        self.history_cursor = None;
        self.blocks.begin_submission(submitted);
        SubmitOutcome::Submitted
    }

    /// `Ctrl+C`: always signals; restores an outstanding submission instead of
    /// dropping it, and clears the editor at an ordinary prompt.
    pub fn interrupt(&mut self) {
        if let Err(error) = self.pty.write(&[0x03]) {
            warn!(error = %error, "editor interrupt failed");
        }
        let state = self.editor_state();
        match state {
            EditorState::AwaitingStart => {
                if let Some(text) = self.blocks.abort_pending() {
                    self.editor.set_text(&text);
                }
                self.history_cursor = None;
            }
            EditorState::Idle => {
                self.editor.clear();
                self.blocks.clear_feedback();
                self.history_cursor = None;
            }
            EditorState::DisabledCold | EditorState::Running | EditorState::DisabledClosed => {}
        }
        if matches!(state, EditorState::Idle | EditorState::AwaitingStart) {
            debug!(
                restored = state == EditorState::AwaitingStart,
                "editor canceled"
            );
        }
    }

    /// Sanitized clipboard text for the locked editor states (BR-9).
    pub fn paste_to_pty(&mut self, text: &str) -> Result<()> {
        if !matches!(
            self.editor_state(),
            EditorState::AwaitingStart | EditorState::Running
        ) {
            return Ok(());
        }
        let sanitized = sanitize_paste(text);
        if sanitized.is_empty() {
            return Ok(());
        }
        self.pty.write(sanitized.as_bytes())
    }

    /// Encodes a key for the foreground application using the modes the
    /// surface negotiated.
    pub fn encode_key(&self, chord: &KeyChord) -> Option<Vec<u8>> {
        encode_key(chord, self.surface.modes())
    }

    /// Scrolls the editor window by whole visual lines while it is idle.
    pub fn scroll_editor(&mut self, lines: isize) -> bool {
        if self.editor_state() != EditorState::Idle {
            return false;
        }
        self.editor.scroll_lines(lines)
    }

    /// Recalls the next older history entry; the caret lands at its end.
    pub fn history_up(&mut self) -> bool {
        if self.editor_state() != EditorState::Idle {
            return false;
        }
        let len = self.blocks.history_len();
        let next = match self.history_cursor {
            None => 0,
            Some(index) if index + 1 < len => index + 1,
            Some(_) => return false,
        };
        let Some(entry) = self.blocks.history_entry(next) else {
            return false;
        };
        let entry = entry.to_owned();
        self.history_cursor = Some(next);
        trace!(index = next, "editor history recall");
        self.editor.set_text(&entry);
        true
    }

    /// Moves toward newer history; index zero clears the editor.
    pub fn history_down(&mut self) -> bool {
        if self.editor_state() != EditorState::Idle {
            return false;
        }
        let Some(index) = self.history_cursor else {
            return false;
        };
        if index == 0 {
            self.history_cursor = None;
            self.editor.clear();
            return true;
        }
        let next = index - 1;
        let Some(entry) = self.blocks.history_entry(next) else {
            return false;
        };
        let entry = entry.to_owned();
        self.history_cursor = Some(next);
        trace!(index = next, "editor history recall");
        self.editor.set_text(&entry);
        true
    }

    /// Refills the editor from a restored submission after ingest/finalize.
    fn reconcile(&mut self) {
        if let Some(text) = self.blocks.take_restored() {
            self.editor.set_text(&text);
            self.history_cursor = None;
        }
    }

    /// Raw bytes to the PTY. Presentation never writes the PTY directly.
    pub fn write_keystrokes(&mut self, bytes: &[u8]) -> Result<()> {
        self.pty.write(bytes)
    }

    /// Immediately forwards to the PTY (SIGWINCH) and shifts the surface's
    /// wrap width for incoming bytes. No debounce; stored rows are untouched.
    pub fn resize(&mut self, size: TerminalSize) -> Result<()> {
        self.surface.resize(size);
        self.pty.resize(size)
    }
}
