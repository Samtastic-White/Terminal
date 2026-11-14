use tracing::trace;

use crate::application::block::{BlockService, Timestamp};
use crate::domain::block::{Block, BlockId, CopyKind};
use crate::domain::output::Row;
use crate::domain::pty::PtyPort;
use crate::domain::rows::BoundedRows;
use crate::domain::semantic::ShellPhase;
use crate::domain::size::TerminalSize;
use crate::domain::terminal_surface::{SurfaceItem, TerminalSurfacePort};

/// Orchestrates one session: keystroke routing, byte ingestion, and resize.
/// It never touches the PTY implementation, the interpreter implementation,
/// or gpui.
pub struct SessionService {
    pty: Box<dyn PtyPort>,
    surface: Box<dyn TerminalSurfacePort>,
    blocks: BlockService,
}

impl SessionService {
    pub fn new(pty: Box<dyn PtyPort>, surface: Box<dyn TerminalSurfacePort>) -> Self {
        Self {
            pty,
            surface,
            blocks: BlockService::default(),
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
    }

    /// Flushes the interpreter's last partial row and then finalizes the
    /// session: a still-running block becomes Interrupted. Idempotent; safe to
    /// call after the reader channel disconnected.
    pub fn finalize_stream(&mut self) {
        let items = self.surface.flush();
        self.blocks.handle_items(items, Timestamp::now());
        self.blocks.session_ended(Timestamp::now());
    }

    /// Blocks in stream order, oldest first.
    pub fn blocks(&self) -> &[Block] {
        self.blocks.blocks()
    }

    /// Committed rows of the prompt currently being drawn.
    pub fn prompt_rows(&self) -> &BoundedRows {
        self.blocks.prompt_rows()
    }

    /// The surface row still being built, exposed only in a rendered window.
    /// Committed rows never pass through here, so drawing the live row cannot
    /// duplicate a line.
    pub fn live_row(&self) -> Option<Row> {
        self.blocks.live_row(self.surface.active_row())
    }

    /// Current shell position in the block lifecycle.
    pub fn phase(&self) -> ShellPhase {
        self.blocks.phase()
    }

    /// Plain-text extraction for one block; `None` if the id was evicted.
    pub fn copy_block(&self, id: BlockId, kind: CopyKind) -> Option<String> {
        self.blocks.copy_block(id, kind)
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
}
