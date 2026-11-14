//! Session orchestration: keystroke routing, byte ingestion, and resize.

mod service;

#[cfg(test)]
mod tests;

pub use service::SessionService;
