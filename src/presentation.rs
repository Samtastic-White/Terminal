//! Presentation layer: GPUI views, components, and rendering.
//!
//! Driving adapter: translates GPUI events into application calls. It never
//! holds terminal state or touches the PTY.

pub mod components;
pub mod session_view;
pub mod theme;
