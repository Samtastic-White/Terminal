//! `PtyPort` — Domain port for the child shell's pseudo-terminal.

use crate::common::errors::Result;

/// Terminal size in (cols, rows).
///
/// Construction clamps both dimensions to at least 1: the kernel treats a
/// 0-sized winsize as unset and SIGWINCH payloads would be nonsense
/// (spec BR-7 prep).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    /// Clamps zero inputs to 1 (PTY sizes are never 0).
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }
}

/// Contract for spawning, writing to, and resizing the child
/// pseudo-terminal.
///
/// DRIVEN port: Infrastructure implements it (`design.md` § 2);
/// Presentation never touches the PTY directly (architecture Common
/// Mistake #3).
pub trait PtyPort {
    /// Spawn the child shell. Idempotent per session: a second call is a
    /// no-op while a live session exists (spec BR-1).
    fn spawn(&mut self, size: PtySize) -> Result<()>;

    /// Write raw bytes (keystroke passthrough, spec BR-8). Short writes
    /// are completed by the adapter before returning.
    fn write(&mut self, bytes: &[u8]) -> Result<()>;

    /// Resize cols/rows; the kernel delivers `SIGWINCH` to the foreground
    /// process group (spec BR-7). Immediate semantics: callers must not
    /// debounce.
    fn resize(&mut self, size: PtySize) -> Result<()>;

    /// True while the child is running; false after EOF/death was
    /// observed (spec BR-9 finalization source).
    fn is_alive(&self) -> bool;
}
