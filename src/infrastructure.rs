//! Infrastructure layer: driven adapters implementing the domain ports.
//!
//! Real-world I/O and OS communication live here. No business rules; never
//! blocks the UI thread.

pub mod parser;
pub mod pty;
