//! Application-wide error type.
//!
//! Infrastructure failures convert into these variants at the infrastructure
//! boundary; Domain models impossible states away with types and never
//! produces errors.

use std::fmt;

/// Fallible APIs return `Result<T, TerminalError>` and propagate with `?`.
#[derive(Debug)]
pub enum TerminalError {
    /// PTY open, spawn, write, or resize failure.
    Pty(String),
    /// Bounded-channel plumbing failure (receiver gone, closed early).
    #[allow(dead_code)]
    Channel(String),
}

impl fmt::Display for TerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pty(message) => write!(f, "pty error: {message}"),
            Self::Channel(message) => write!(f, "channel error: {message}"),
        }
    }
}

impl std::error::Error for TerminalError {}

/// Fallible API result alias for the whole application.
pub type Result<T> = std::result::Result<T, TerminalError>;
