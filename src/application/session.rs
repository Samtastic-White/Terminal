//! Session orchestration: keystroke routing, byte ingestion, and resize.

mod service;

#[cfg(test)]
mod tests;

pub use service::SessionService;
// Presentation key dispatch and the editor dock consume these in T6.
#[allow(unused_imports)]
pub use service::{EditCommand, EditorState, SubmitOutcome};
