//! Infrastructure layer: Driven Adapters implementing the Domain ports.
//!
//! Real-world I/O and OS communication live here
//! (`knowledge/architecture.md` § Infrastructure). No business rules;
//! never blocks the UI thread (`AGENTS.md` § 9).

pub mod parser;
pub mod pty;
