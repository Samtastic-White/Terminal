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
    pub prompt_max_rows: usize,
    pub silence_max_rows: usize,
}

impl Default for BlockLimits {
    fn default() -> Self {
        Self {
            block_max_rows: 4_000,
            session_max_rows: 8_000,
            session_max_blocks: 128,
            prompt_max_rows: 100,
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
    prompt_rows: BoundedRows,
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
            prompt_rows: BoundedRows::new(limits.prompt_max_rows),
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

    /// Committed rows of the prompt currently being drawn.
    pub fn prompt_rows(&self) -> &BoundedRows {
        &self.prompt_rows
    }

    /// Live surface row, exposed only while it belongs to a rendered region.
    pub fn live_row(
        &self,
        surface_row: Option<crate::domain::output::Row>,
    ) -> Option<crate::domain::output::Row> {
        match self.machine.phase() {
            ShellPhase::Running | ShellPhase::Prompting { prompt_open: true } => surface_row,
            ShellPhase::Cold
            | ShellPhase::Prompting { prompt_open: false }
            | ShellPhase::Closed => None,
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
        self.closed = true;
        self.prompt_rows.clear();
        self.silence_rows.clear();
    }

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
            ShellPhase::Prompting { prompt_open: true } => {
                self.prompt_rows.push(row);
            }
            ShellPhase::Prompting { prompt_open: false } | ShellPhase::Cold => {
                self.silence_rows.push(row);
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
            SemanticEvent::PromptStart
            | SemanticEvent::PromptEnd
            | SemanticEvent::CommandStart
            | SemanticEvent::CommandFinished { .. }
            | SemanticEvent::TerminalReset => {}
        }
    }

    fn apply_transitions(&mut self, transitions: Vec<Transition>, at: Timestamp) {
        for transition in transitions {
            match transition {
                Transition::PromptOpened => self.prompt_rows.clear(),
                Transition::BlockOpened => {
                    self.prompt_rows.clear();
                    let id = BlockId::new(self.next_id);
                    self.next_id = self.next_id.wrapping_add(1);
                    self.blocks.push(Block::running(
                        id,
                        self.last_cwd.clone(),
                        at.wall,
                        at.mono,
                        self.limits.block_max_rows,
                    ));
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
                Transition::Closed => {
                    self.closed = true;
                    self.prompt_rows.clear();
                    self.silence_rows.clear();
                }
            }
        }
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
