//! Domain layer: pure entities, state machines, and Port traits.
//!
//! This layer never imports `gpui`, `portable_pty`, `rusqlite`, or OS crates;
//! it stays 100% headless-testable.

pub mod output;
pub mod pty;
pub mod size;
pub mod terminal_surface;
