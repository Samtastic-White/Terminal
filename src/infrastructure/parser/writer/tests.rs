//! Writer unit tests, grouped by area: text, SGR, erase, OSC.

mod erase;
mod modes;
mod osc;
mod sgr;
mod text;

use std::path::PathBuf;

use super::BlockTextWriter;
use crate::domain::output::{Color, Row};
use crate::domain::semantic::SemanticEvent;
use crate::domain::terminal_surface::SurfaceItem;

fn rows_of(cols: u16, chunks: &[&[u8]]) -> Vec<Row> {
    let mut writer = BlockTextWriter::new(cols);
    for chunk in chunks {
        writer.write(chunk);
    }
    writer.flush_active_row();
    writer.take_committed_rows()
}

fn text(row: &Row) -> String {
    row.spans.iter().map(|span| span.text.as_str()).collect()
}

fn items_of(chunks: &[&[u8]]) -> Vec<SurfaceItem> {
    let mut writer = BlockTextWriter::new(80);
    for chunk in chunks {
        writer.write(chunk);
    }
    writer.take_items()
}

fn semantic(event: SemanticEvent) -> SurfaceItem {
    SurfaceItem::Semantic(event)
}
