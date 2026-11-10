//! `PtyPort` — Domain port for the child shell's pseudo-terminal.

use crate::common::errors::Result;
use crate::domain::size::TerminalSize;

/// Contract for spawning, writing to, and resizing the child pseudo-terminal.
///
/// Driven port: infrastructure implements it; presentation never touches the
/// PTY directly.
pub trait PtyPort {
    /// Idempotent per session: a second call is a no-op while a live session
    /// exists.
    fn spawn(&mut self, size: TerminalSize) -> Result<()>;

    /// Writes raw keystroke bytes. Short writes are completed by the adapter
    /// before returning.
    fn write(&mut self, bytes: &[u8]) -> Result<()>;

    /// Resize delivers `SIGWINCH` to the foreground process group. Callers
    /// must not debounce.
    fn resize(&mut self, size: TerminalSize) -> Result<()>;

    /// True while the child is running; false after EOF or child death.
    fn is_alive(&self) -> bool;
}
