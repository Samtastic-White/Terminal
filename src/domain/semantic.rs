//! Semantic shell events and the block state machine.

mod event;
mod machine;

#[cfg(test)]
mod tests;

pub use event::SemanticEvent;
pub use machine::{BlockStateMachine, ShellPhase, Transition};
