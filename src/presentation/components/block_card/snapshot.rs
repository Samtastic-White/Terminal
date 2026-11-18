//! Header snapshot: command, status label, tone, and meta line computed
//! before any GPUI element exists, so it is unit-testable.

#[cfg(test)]
use std::time::SystemTime;

use crate::application::block::format_duration;
#[cfg(test)]
use crate::application::block::format_relative;
use crate::domain::block::{Block, BlockStatus};

/// Semantic tone of a block state, independent of the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Running,
    Success,
    Failure,
    Interrupted,
}

/// Header data computed before any GPUI element exists, so it is unit-testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardSnapshot {
    pub command: String,
    pub status_label: String,
    pub tone: StatusTone,
    pub meta: String,
    pub has_body: bool,
}

/// State pill text. Interrupted uses the protocol's grey dash.
pub fn status_label(status: BlockStatus) -> String {
    match status {
        BlockStatus::Running => "running".into(),
        BlockStatus::Succeeded => "ok".into(),
        BlockStatus::Failed { exit_code } => format!("exit {exit_code}"),
        BlockStatus::Interrupted => "—".into(),
    }
}

pub fn status_tone(status: BlockStatus) -> StatusTone {
    match status {
        BlockStatus::Running => StatusTone::Running,
        BlockStatus::Succeeded => StatusTone::Success,
        BlockStatus::Failed { .. } => StatusTone::Failure,
        BlockStatus::Interrupted => StatusTone::Interrupted,
    }
}

/// First logical line of a submitted command, ellipsized when the command
/// spans more lines. Clipboard extraction always keeps the full text.
pub fn command_preview(block: &Block) -> String {
    let Some(command) = block.command() else {
        return "…".into();
    };
    match command.split_once('\n') {
        Some((first, _)) => format!("{first}…"),
        None => command.to_owned(),
    }
}

/// Everything the header paints for one block, plus whether a body exists.
#[cfg(test)]
pub fn card_snapshot(block: &Block, now: SystemTime) -> CardSnapshot {
    card_snapshot_with_relative(block, format_relative(block.started_wall(), now))
}

/// Snapshot with a caller-cached relative label, so a running block does not
/// reformat its age every frame.
pub fn card_snapshot_with_relative(block: &Block, relative: String) -> CardSnapshot {
    let meta = match block.finished() {
        Some((_, duration)) => format!("{relative} · {}", format_duration(duration)),
        None => relative,
    };
    CardSnapshot {
        command: command_preview(block),
        status_label: status_label(block.status()),
        tone: status_tone(block.status()),
        meta,
        has_body: !block.output().is_empty(),
    }
}
