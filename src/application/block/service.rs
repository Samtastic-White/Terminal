//! Block orchestration: phase routing, bounded buffers and copy extraction.

use std::path::PathBuf;

use tracing::{debug, trace};

use super::time::Timestamp;
use crate::domain::block::{Block, BlockId, CopyKind};
use crate::domain::rows::BoundedRows;
use crate::domain::semantic::{BlockStateMachine, SemanticEvent, ShellPhase, Transition};
use crate::domain::terminal_surface::SurfaceItem;

/// Row and block budgets for one session.
#[derive(Debug, Clone, Copy)]
pub struct BlockLimits {
    pub block_max_rows: usize,
    pub session_max_rows: usize,
    pub session_max_blocks: usize,
    pub feedback_max_rows: usize,
    pub silence_max_rows: usize,
}

impl Default for BlockLimits {
    fn default() -> Self {
        Self {
            block_max_rows: 4_000,
            session_max_rows: 8_000,
            session_max_blocks: 128,
            feedback_max_rows: 3,
            silence_max_rows: 32,
        }
    }
}

/// Owns block state for one session. The renderer sees finished blocks and
/// the live prompt; raw PTY bytes never reach this type.
#[derive(Debug)]
pub struct BlockService {
    limits: BlockLimits,
    machine: BlockStateMachine,
    blocks: Vec<Block>,
    pending: Option<String>,
    awaiting_start: bool,
    feedback: BoundedRows,
    restored: Option<String>,
    silence_rows: BoundedRows,
    last_cwd: Option<PathBuf>,
    total_rows: usize,
    next_id: u64,
    closed: bool,
}

impl Default for BlockService {
    fn default() -> Self {
        Self::new(BlockLimits::default())
    }
}

impl BlockService {
    pub fn new(limits: BlockLimits) -> Self {
        Self {
            limits,
            machine: BlockStateMachine::new(),
            blocks: Vec::new(),
            pending: None,
            awaiting_start: false,
            feedback: BoundedRows::new(limits.feedback_max_rows),
            restored: None,
            silence_rows: BoundedRows::new(limits.silence_max_rows),
            last_cwd: None,
            total_rows: 0,
            next_id: 0,
            closed: false,
        }
    }

    /// Current shell position in the block lifecycle.
    pub fn phase(&self) -> ShellPhase {
        self.machine.phase()
    }

    /// Blocks in stream order, oldest first.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Rows received while a submission waits for `C` or `D`.
    pub fn feedback_rows(&self) -> &BoundedRows {
        &self.feedback
    }

    /// Text submitted but not yet attached to a block.
    // The editor dock derives its locked content from this in T6.
    #[allow(dead_code)]
    pub fn pending_text(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    /// True from submission until the next `A` unlocks the editor.
    pub fn awaiting_start(&self) -> bool {
        self.awaiting_start
    }

    /// True after the shell ended or the terminal reset.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Starts the submission lifecycle: the text is retained until `C`
    /// attaches it or `D` restores it.
    pub fn begin_submission(&mut self, text: String) {
        self.pending = Some(text);
        self.awaiting_start = true;
        self.feedback.clear();
    }

    /// Moves a restored submission out for the editor to refill.
    pub fn take_restored(&mut self) -> Option<String> {
        self.restored.take()
    }

    /// Drops a pending submission on explicit `Ctrl+C` and returns its text.
    pub fn abort_pending(&mut self) -> Option<String> {
        self.awaiting_start = false;
        self.feedback.clear();
        self.pending.take()
    }

    /// Clears transient shell feedback after the user edits the text.
    pub fn clear_feedback(&mut self) {
        self.feedback.clear();
    }

    /// Newest-first command history derived from opened blocks.
    pub fn history_len(&self) -> usize {
        let mut seen: Option<&str> = None;
        let mut len = 0;
        for block in self.blocks.iter().rev() {
            if block.is_running() {
                continue;
            }
            let Some(command) = block.command() else {
                continue;
            };
            if command.is_empty() || seen == Some(command) {
                continue;
            }
            seen = Some(command);
            len += 1;
        }
        len
    }

    /// One history entry, zero-based and newest first.
    pub fn history_entry(&self, index: usize) -> Option<&str> {
        let mut seen: Option<&str> = None;
        let mut rank = 0;
        for block in self.blocks.iter().rev() {
            if block.is_running() {
                continue;
            }
            let Some(command) = block.command() else {
                continue;
            };
            if command.is_empty() || seen == Some(command) {
                continue;
            }
            seen = Some(command);
            if rank == index {
                return Some(command);
            }
            rank += 1;
        }
        None
    }

    /// Live surface row, exposed only while it belongs to a running block.
    pub fn live_row(
        &self,
        surface_row: Option<crate::domain::output::Row>,
    ) -> Option<crate::domain::output::Row> {
        match self.machine.phase() {
            ShellPhase::Running => surface_row,
            ShellPhase::Cold | ShellPhase::Prompting { .. } | ShellPhase::Closed => None,
        }
    }

    /// Plain-text extraction for one block; `None` if the id was evicted.
    pub fn copy_block(&self, id: BlockId, kind: CopyKind) -> Option<String> {
        self.blocks
            .iter()
            .find(|block| block.id() == id)
            .map(|block| block.copy(kind))
    }

    /// Applies rows and markers in stream order, routing rows by the phase
    /// that was active before the item.
    pub fn handle_items(&mut self, items: Vec<SurfaceItem>, at: Timestamp) {
        for item in items {
            if self.closed {
                break;
            }
            match item {
                SurfaceItem::Row(row) => self.push_row(row),
                SurfaceItem::Semantic(event) => self.handle_event(event, at),
            }
        }
    }

    /// EOF or terminal reset: the running block has no `D` marker.
    pub fn session_ended(&mut self, at: Timestamp) {
        if !self.closed {
            let transitions = self.machine.on(SemanticEvent::TerminalReset);
            self.apply_transitions(transitions, at);
        }
        if let Some(block) = self.blocks.last_mut().filter(|block| block.is_running()) {
            block.interrupt(at.wall, at.mono);
            debug!(
                block_id = block.id().get(),
                duration_ms = block
                    .finished()
                    .map(|(_, duration)| duration.as_millis() as u64)
                    .unwrap_or(0),
                "block interrupted"
            );
        }
        self.close();
    }

    /// Rows at the prompt are feedback while a submission is pending and
    /// silence otherwise.
    fn push_row(&mut self, row: crate::domain::output::Row) {
        match self.machine.phase() {
            ShellPhase::Running => {
                if let Some(block) = self.blocks.last_mut().filter(|block| block.is_running()) {
                    if block.push_row(row) {
                        self.total_rows = self.total_rows.saturating_sub(1);
                    }
                    self.total_rows += 1;
                }
                self.enforce_budgets();
            }
            ShellPhase::Prompting { .. } | ShellPhase::Cold => {
                if self.pending.is_some() {
                    self.feedback.push(row);
                } else {
                    self.silence_rows.push(row);
                }
            }
            ShellPhase::Closed => {}
        }
    }

    fn handle_event(&mut self, event: SemanticEvent, at: Timestamp) {
        let transitions = self.machine.on(event.clone());
        self.apply_transitions(transitions, at);

        match event {
            SemanticEvent::DirectoryChanged { path } => {
                self.last_cwd = Some(path);
            }
            SemanticEvent::CommandLine { text } => {
                if let Some(block) = self.blocks.last_mut().filter(|block| block.is_running())
                    && block.command().is_none()
                {
                    block.set_command(text);
                }
            }
            SemanticEvent::PromptStart => {
                // A prompt redraw after a failed submission produces no
                // machine transition, but it must still release the editor.
                if self.awaiting_start && self.pending.is_none() {
                    self.awaiting_start = false;
                }
            }
            SemanticEvent::PromptEnd
            | SemanticEvent::CommandStart
            | SemanticEvent::TerminalReset => {}
            SemanticEvent::CommandFinished { .. } => self.restore_unstarted_command(),
        }
    }

    /// `D` while still prompting means the command never opened a block
    /// (syntax error, continuation abort): keep the feedback and hand the
    /// pending text back to the editor.
    fn restore_unstarted_command(&mut self) {
        if matches!(self.machine.phase(), ShellPhase::Prompting { .. })
            && let Some(text) = self.pending.take()
        {
            self.restored = Some(text);
        }
    }

    fn apply_transitions(&mut self, transitions: Vec<Transition>, at: Timestamp) {
        for transition in transitions {
            match transition {
                Transition::PromptOpened => {
                    // The raw `A` item owns the unlock: a redrawn prompt emits
                    // no transition and must release the editor too.
                }
                Transition::BlockOpened => {
                    let pending = self.pending.take();
                    let id = BlockId::new(self.next_id);
                    self.next_id = self.next_id.wrapping_add(1);
                    let mut block = Block::running(
                        id,
                        self.last_cwd.clone(),
                        at.wall,
                        at.mono,
                        self.limits.block_max_rows,
                    );
                    if let Some(text) = pending {
                        block.set_command(text);
                    }
                    self.blocks.push(block);
                    self.feedback.clear();
                    debug!(block_id = id.get(), "block opened");
                    self.enforce_budgets();
                }
                Transition::BlockCompleted { exit_code } => {
                    if let Some(block) = self.blocks.last_mut().filter(|block| block.is_running()) {
                        block.complete(exit_code, at.wall, at.mono);
                        debug!(
                            block_id = block.id().get(),
                            exit_code,
                            duration_ms = block
                                .finished()
                                .map(|(_, duration)| duration.as_millis() as u64)
                                .unwrap_or(0),
                            "block completed"
                        );
                    }
                }
                Transition::BlockInterrupted => {
                    if let Some(block) = self.blocks.last_mut().filter(|block| block.is_running()) {
                        block.interrupt(at.wall, at.mono);
                        debug!(
                            block_id = block.id().get(),
                            duration_ms = block
                                .finished()
                                .map(|(_, duration)| duration.as_millis() as u64)
                                .unwrap_or(0),
                            "block interrupted"
                        );
                    }
                }
                Transition::Closed => self.close(),
            }
        }
    }

    /// Terminal end: pending text becomes restorable, transient rings clear.
    fn close(&mut self) {
        if let Some(text) = self.pending.take() {
            self.restored = Some(text);
        }
        self.awaiting_start = false;
        self.feedback.clear();
        self.silence_rows.clear();
        self.closed = true;
    }

    /// Oldest completed blocks are evicted first; the running block is never
    /// removed because its own cap keeps it under the session budget.
    fn enforce_budgets(&mut self) {
        while self.total_rows > self.limits.session_max_rows {
            let Some(index) = self.blocks.iter().position(|block| !block.is_running()) else {
                break;
            };
            self.remove_block(index);
        }
        while self.blocks.len() > self.limits.session_max_blocks {
            let Some(index) = self.blocks.iter().position(|block| !block.is_running()) else {
                break;
            };
            self.remove_block(index);
        }
    }

    fn remove_block(&mut self, index: usize) {
        let removed = self.blocks.remove(index);
        let rows = removed.output().len();
        self.total_rows = self.total_rows.saturating_sub(rows);
        trace!(block_id = removed.id().get(), rows, "block evicted");
    }
}
