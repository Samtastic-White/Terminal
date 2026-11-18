//! `TerminalSurfacePort` — Domain port for the terminal engine.
//!
//! The engine sits behind this port so the application consumes the byte
//! stream while depending only on Domain.
//!
//! `SessionService` is the only holder. Blocks consume interpreted `Row`s and
//! must never receive `&dyn TerminalSurfacePort`.
//!
//! Do not add speculative viewport, cursor, or alternate-screen methods here.
//! `active_row` is the one live-row primitive: it exposes what the writer is
//! still building so prompts and typed input render before a newline commits
//! them. It is not a viewport contract.

use crate::domain::keymap::TerminalModes;
use crate::domain::output::Row;
use crate::domain::semantic::SemanticEvent;
use crate::domain::size::TerminalSize;

/// One committed output of the surface, in stream order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceItem {
    Row(Row),
    Semantic(SemanticEvent),
}

pub trait TerminalSurfacePort {
    /// UTF-8 multibyte sequences split across calls reassemble inside the
    /// implementation. Returns the rows and semantic events committed by this
    /// chunk, plus any previously undelivered items, in stream order. A row
    /// still being built when a recognized marker arrives precedes its event.
    fn interpret(&mut self, bytes: &[u8]) -> Vec<SurfaceItem>;

    /// Newly arriving bytes wrap at the new width; stored rows are never
    /// re-wrapped.
    fn resize(&mut self, size: TerminalSize);

    /// Non-destructive snapshot of the row still being built, or `None` when
    /// no cell is written. Reading it never commits or clears the row; only a
    /// newline, auto-wrap, or `flush` retires it.
    fn active_row(&self) -> Option<Row>;

    /// Private modes the foreground application negotiated in this stream.
    /// Reading them never changes the stream.
    // The session encoder becomes the production reader in T4; until then the
    // parser and the test mock are the only implementors.
    #[allow(dead_code)]
    fn modes(&self) -> TerminalModes;

    /// Flushes the partially built row and every not-yet-drained item. A
    /// fully unwritten line fabricates no row.
    fn flush(&mut self) -> Vec<SurfaceItem>;
}
