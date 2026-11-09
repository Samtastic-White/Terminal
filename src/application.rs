//! Application layer: orchestrates business use cases. Depends only on
//! Domain (`knowledge/architecture.md` § Layer Responsibilities) — never
//! on gpui or Infrastructure directly.

pub mod session;
