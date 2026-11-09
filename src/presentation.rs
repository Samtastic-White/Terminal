//! Presentation layer: GPUI views, components, and rendering.
//!
//! Driving Adapter — translates GPUI events into Application layer calls.
//! Never contains terminal state logic or PTY operations.
//! (`knowledge/architecture.md` § Layer Responsibilities)

pub mod components;
pub mod session_view;
pub mod theme;
