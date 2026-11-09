//! `TextInterpreterPort` — Domain port for interpreting raw child-shell
//! bytes into styled rows.
//!
//! DRIVEN port (design.md § writer/data model): Phase 1 implements it
//! with the bounded `vte::Perform` subset (`BlockTextWriter`); Phase 4
//! plugs the full grid engine under the same port. Keeping the
//! interpreter behind a port is what lets `src/application/` consume the
//! byte stream while depending only on Domain (dependency direction).

use crate::domain::output::Row;

pub trait TextInterpreterPort {
    /// Interpret one raw chunk. UTF-8 multibyte sequences split across
    /// calls reassemble inside the implementation (spec BR-5). Returns
    /// the rows committed by this (and any previously undelivered) chunk.
    fn interpret(&mut self, bytes: &[u8]) -> Vec<Row>;

    /// Resize path (spec BR-7): newly arriving bytes wrap the NEW width;
    /// already stored rows are not re-wrapped by anybody (no reflow in
    /// this phase).
    fn resize_cols(&mut self, cols: u16);

    /// EOF finalization (spec BR-9): flush the partially built row —
    /// never fabricating one from a fully unwritten line — and deliver
    /// every not-yet-drained row.
    fn flush(&mut self) -> Vec<Row>;
}
