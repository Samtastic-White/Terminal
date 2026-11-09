//! Domain layer: pure entities, state machines, and Port traits.
//!
//! Inviolable rule (`knowledge/architecture.md` § Layer Responsibilities /
//! `AGENTS.md` § 9): this layer NEVER imports `gpui`, `portable_pty`,
//! `rusqlite`, or OS crates. It must stay 100% headless-testable.

pub mod interpreter;
pub mod output;
pub mod pty;
