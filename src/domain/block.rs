//! Command block entity: lifecycle, bounded output and copy extraction.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::domain::output::Row;
use crate::domain::rows::BoundedRows;

/// Stable identity for a block within one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(u64);

impl BlockId {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Final or in-flight state of a command block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStatus {
    Running,
    Succeeded,
    Failed { exit_code: u32 },
    Interrupted,
}

/// Portion of a block extracted for the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    Command,
    Output,
    WholeBlock,
}

/// A single command execution with its bounded output.
#[derive(Debug, Clone)]
pub struct Block {
    id: BlockId,
    command: Option<String>,
    /// Block metadata; no production reader yet.
    #[allow(dead_code)]
    cwd: Option<PathBuf>,
    started_wall: SystemTime,
    started_mono: Instant,
    finished: Option<(SystemTime, Duration)>,
    status: BlockStatus,
    output: BoundedRows,
    output_revision: u64,
}

impl Block {
    /// Creates a block that is already executing; a block exists only after C.
    pub fn running(
        id: BlockId,
        cwd: Option<PathBuf>,
        started_wall: SystemTime,
        started_mono: Instant,
        max_rows: usize,
    ) -> Self {
        Self {
            id,
            command: None,
            cwd,
            started_wall,
            started_mono,
            finished: None,
            status: BlockStatus::Running,
            output: BoundedRows::new(max_rows),
            output_revision: 0,
        }
    }

    pub fn id(&self) -> BlockId {
        self.id
    }

    pub fn command(&self) -> Option<&str> {
        self.command.as_deref()
    }

    pub fn set_command(&mut self, command: String) {
        self.command = Some(command);
    }

    /// Block metadata; no production reader yet.
    #[allow(dead_code)]
    pub fn cwd(&self) -> Option<&PathBuf> {
        self.cwd.as_ref()
    }

    pub fn started_wall(&self) -> SystemTime {
        self.started_wall
    }

    pub fn status(&self) -> BlockStatus {
        self.status
    }

    pub fn is_running(&self) -> bool {
        self.status == BlockStatus::Running
    }

    /// Wall-clock instant and monotonic duration of the last transition.
    pub fn finished(&self) -> Option<(SystemTime, Duration)> {
        self.finished
    }

    pub fn output(&self) -> &BoundedRows {
        &self.output
    }

    /// Monotonic counter bumped whenever the stored output changes.
    pub fn output_revision(&self) -> u64 {
        self.output_revision
    }

    /// Total rows ever pushed, including rows already evicted by the cap.
    pub fn output_total(&self) -> u64 {
        self.output.pushed()
    }

    /// Appends an output row, returning true when the block evicted its oldest.
    pub fn push_row(&mut self, row: Row) -> bool {
        let evicted = self.output.push(row);
        self.output_revision = self.output_revision.wrapping_add(1);
        evicted
    }

    /// Completes the block with the shell's exit status.
    pub fn complete(&mut self, exit_code: u32, wall: SystemTime, mono: Instant) {
        self.status = if exit_code == 0 {
            BlockStatus::Succeeded
        } else {
            BlockStatus::Failed { exit_code }
        };
        self.finished = Some((wall, mono.saturating_duration_since(self.started_mono)));
    }

    /// Marks the block interrupted, keeping its elapsed time and output.
    pub fn interrupt(&mut self, wall: SystemTime, mono: Instant) {
        self.status = BlockStatus::Interrupted;
        self.finished = Some((wall, mono.saturating_duration_since(self.started_mono)));
    }

    /// Extracts plain text for the clipboard; ANSI styling never reaches here.
    pub fn copy(&self, kind: CopyKind) -> String {
        match kind {
            CopyKind::Command => self.command.clone().unwrap_or_default(),
            CopyKind::Output => self.output_text(),
            CopyKind::WholeBlock => match (self.command(), self.output.is_empty()) {
                (Some(command), false) => {
                    let mut text = String::with_capacity(command.len() + 1);
                    text.push_str(command);
                    text.push('\n');
                    text.push_str(&self.output_text());
                    text
                }
                (Some(command), true) => command.to_owned(),
                (None, _) => self.output_text(),
            },
        }
    }

    fn output_text(&self) -> String {
        let rows: Vec<String> = self
            .output
            .iter()
            .map(|row| {
                let mut text = String::new();
                for span in &row.spans {
                    text.push_str(&span.text);
                }
                text
            })
            .collect();
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::output::{Span, TextStyle};

    fn row(text: &str) -> Row {
        Row {
            spans: vec![Span {
                style: TextStyle::default(),
                text: text.into(),
            }],
        }
    }

    fn multi_span_row(parts: &[&str]) -> Row {
        Row {
            spans: parts
                .iter()
                .map(|text| Span {
                    style: TextStyle::default(),
                    text: (*text).into(),
                })
                .collect(),
        }
    }

    fn running() -> Block {
        Block::running(
            BlockId::new(1),
            Some(PathBuf::from("/tmp")),
            SystemTime::UNIX_EPOCH,
            Instant::now(),
            4,
        )
    }

    #[test]
    fn a_new_block_starts_running_without_a_command() {
        let block = running();

        assert_eq!(block.status(), BlockStatus::Running);
        assert!(block.command().is_none());
        assert_eq!(block.cwd(), Some(&PathBuf::from("/tmp")));
        assert!(block.finished().is_none());
        assert_eq!(block.output_revision(), 0);
    }

    #[test]
    fn pushing_rows_bumps_the_revision() {
        let mut block = running();

        block.push_row(row("a"));
        block.push_row(row("b"));

        assert_eq!(block.output_revision(), 2);
        assert_eq!(block.output().len(), 2);
    }

    #[test]
    fn output_evicts_oldest_rows_at_the_block_cap() {
        let mut block = running();

        for text in ["a", "b", "c", "d", "e"] {
            block.push_row(row(text));
        }

        assert_eq!(block.output().len(), 4);
        assert_eq!(block.copy(CopyKind::Output), "b\nc\nd\ne");
    }

    #[test]
    fn completing_with_zero_succeeds() {
        let mut block = running();

        block.complete(0, SystemTime::UNIX_EPOCH, Instant::now());

        assert_eq!(block.status(), BlockStatus::Succeeded);
        assert!(block.finished().is_some());
    }

    #[test]
    fn completing_with_a_nonzero_code_fails_and_keeps_the_code() {
        let mut block = running();

        block.complete(2, SystemTime::UNIX_EPOCH, Instant::now());

        assert_eq!(block.status(), BlockStatus::Failed { exit_code: 2 });
    }

    #[test]
    fn interrupting_keeps_the_elapsed_time_and_status() {
        let mut block = running();
        block.push_row(row("partial"));

        block.interrupt(SystemTime::UNIX_EPOCH, Instant::now());

        assert_eq!(block.status(), BlockStatus::Interrupted);
        assert!(block.finished().is_some());
        assert_eq!(block.copy(CopyKind::Output), "partial");
    }

    #[test]
    fn command_copy_is_exact_and_empty_without_e() {
        let mut block = running();

        assert_eq!(block.copy(CopyKind::Command), "");

        let command = "printf '100%n' | sed 's/;/:/'";
        block.set_command(command.into());

        assert_eq!(block.copy(CopyKind::Command), command);
    }

    #[test]
    fn output_copy_joins_rows_and_spans_without_styling() {
        let mut block = running();
        block.push_row(multi_span_row(&["hello ", "world"]));
        block.push_row(Row::default());
        block.push_row(row("third"));

        assert_eq!(block.copy(CopyKind::Output), "hello world\n\nthird");
    }

    #[test]
    fn whole_block_copy_prepends_the_command() {
        let mut block = running();
        block.set_command("echo hi".into());
        block.push_row(row("hi"));

        assert_eq!(block.copy(CopyKind::WholeBlock), "echo hi\nhi");
    }

    #[test]
    fn whole_block_copy_is_command_only_without_output() {
        let mut block = running();
        block.set_command("true".into());

        assert_eq!(block.copy(CopyKind::WholeBlock), "true");
    }

    #[test]
    fn whole_block_copy_is_output_only_without_a_command() {
        let mut block = running();
        block.push_row(row("orphan"));

        assert_eq!(block.copy(CopyKind::WholeBlock), "orphan");
    }
}
