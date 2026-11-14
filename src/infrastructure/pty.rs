//! `PtyAdapter` implements the domain `PtyPort` with `portable-pty`.
//!
//! Reads run on a dedicated OS thread. Chunks travel through a bounded
//! `SyncSender` whose `send` blocks when full: the reader parks and the kernel
//! pipe buffer throttles the child. PTY bytes are never dropped. EOF marks the
//! session ended and reaps the child, leaving no zombie.

mod adapter;
mod shell_integration;

#[cfg(test)]
mod tests;

pub use adapter::PtyAdapter;

const PTY_BUFFER_BYTES: usize = 8 * 1024;
/// Bounded channel capacity in chunks (about 512 KiB in flight).
const CHANNEL_CAP: usize = 64;
const ZSH_PATH: &str = "/bin/zsh";
