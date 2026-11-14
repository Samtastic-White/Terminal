//! Block orchestration: bounded buffers, phase routing, copy extraction, and
//! header time formatting.

mod service;
mod time;

#[cfg(test)]
mod tests;

pub use service::BlockService;
pub use time::{Timestamp, format_duration, format_relative};

#[cfg(test)]
pub use service::BlockLimits;
