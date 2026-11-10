//! `TerminalSurfacePort` — Domain port for the terminal engine.
//!
//! The engine sits behind this port so the application consumes the byte
//! stream while depending only on Domain.
//!
//! `SessionService` is the only holder. Blocks consume interpreted `Row`s and
//! must never receive `&dyn TerminalSurfacePort`.
//!
//! Do not add speculative viewport, cursor, or alternate-screen methods here.

use crate::domain::output::Row;
use crate::domain::size::TerminalSize;

pub trait TerminalSurfacePort {
    /// UTF-8 multibyte sequences split across calls reassemble inside the
    /// implementation. Returns the rows committed by this chunk and any
    /// previously undelivered rows.
    fn interpret(&mut self, bytes: &[u8]) -> Vec<Row>;

    /// Newly arriving bytes wrap at the new width; stored rows are never
    /// re-wrapped.
    fn resize(&mut self, size: TerminalSize);

    /// Flushes the partially built row and every not-yet-drained row. A fully
    /// unwritten line fabricates no row.
    fn flush(&mut self) -> Vec<Row>;
}
