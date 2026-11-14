//! Escape-stream interpreter: converts raw child bytes into committed styled
//! rows and semantic events (`SurfaceItem`).

mod writer;

#[cfg(test)]
mod golden;

pub use writer::BlockTextWriter;
