//! Application-wide error type (`knowledge/conventions.md` § Error
//! Handling).
//!
//! Infrastructure failures are converted into these variants at the
//! infrastructure boundary; Domain logic models impossible states away
//! with types and never produces errors.

use std::fmt;

/// Single application error type. Fallible APIs return
/// `Result<T, TerminalError>` and propagate with `?`.
#[derive(Debug)]
pub enum TerminalError {
    /// PTY open / spawn / write / resize failure (`PtyPort`
    /// implementations).
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
