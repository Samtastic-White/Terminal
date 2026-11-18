//! Session view — render surface, key capture, and resize hook.
//!
//! The window bootstrap and infrastructure adapters live in the composition
//! root (`src/main.rs`); this view depends only on the application service and
//! domain types. Presentation never holds a PTY handle.

mod events;
mod keys;
mod render;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::*;

use crate::application::session::SessionService;
use crate::domain::block::BlockId;
use crate::domain::size::TerminalSize;
use crate::presentation::components::block_card::BlockUiState;
use crate::presentation::components::input_editor::InputEditorState;
use crate::presentation::theme::TerminalTheme;

/// Where the most recent non-empty selection lives; copy re-validates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionOrigin {
    Editor,
    Block(BlockId),
}

/// Chunks are parsed in bounded slices so one burst never monopolizes a frame.
const DRAIN_BUDGET: Duration = Duration::from_millis(4);
/// Yield after an exhausted budget; keeps the window painting under load.
const CONTINUE_POLL: Duration = Duration::from_millis(1);

/// One session rendered as styled rows.
pub struct TerminalWindow {
    pub(super) focus_handle: FocusHandle,
    pub(super) session_service: SessionService,
    pty_receiver: Receiver<Vec<u8>>,
    pub(super) theme: TerminalTheme,
    pub(super) scroll: ScrollHandle,
    pub(super) last_terminal_size: Option<TerminalSize>,
    /// Auto-follow is active until the user scrolls back up.
    pub(super) follow_tail: bool,
    /// Per-block body/selection state, pruned when a block is evicted.
    pub(super) block_ui: HashMap<BlockId, Rc<RefCell<BlockUiState>>>,
    /// Last non-empty selection, re-validated when copying.
    pub(super) last_selection: Option<SelectionOrigin>,
    /// Shaped editor layout shared with pointer hit-testing.
    pub(super) editor_layout: Rc<RefCell<InputEditorState>>,
    /// Monospace advance in pixels, measured from the window text system.
    pub(super) editor_char_width: f32,
    /// Pixel remainder accumulated across wheel events.
    pub(super) editor_scroll_accum: f32,
    /// Last pinned card, so state changes are logged once.
    pub(super) pinned_block: Option<BlockId>,
    /// The reader channel closed and the final row was flushed.
    stream_closed: bool,
    pub(super) did_focus: bool,
    /// Foreground drain task; dropping it would stop output ingestion.
    _drain_task: Task<()>,
}

impl TerminalWindow {
    /// Builds the session view around an already spawned session service.
    pub fn new(
        session_service: SessionService,
        pty_receiver: Receiver<Vec<u8>>,
        pty_waker: Receiver<()>,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = TerminalTheme::default();

        // Bounded channel consumer that never runs on the render thread.
        // Draining runs under a short budget; when no chunks are buffered the
        // loop parks on the coalesced reader signal instead of polling, and
        // the blocking wait runs on a background worker.
        let wake_receiver = Arc::new(Mutex::new(pty_waker));
        let _drain_task = cx.spawn(async move |this, cx| {
            let mut wake_closed = false;
            loop {
                let (closed, more) = match this.update(cx, |this, cx| {
                    let (changed, closed, more) = this.drain_output(Instant::now());
                    if changed {
                        cx.notify();
                    }
                    (closed, more)
                }) {
                    Ok(result) => result,
                    Err(_) => break, // view released
                };
                if closed {
                    break;
                }
                if more {
                    cx.background_executor().timer(CONTINUE_POLL).await;
                    continue;
                }
                if wake_closed {
                    // Reader gone and nothing buffered: the final drain above
                    // already saw the disconnect or there is no wake source.
                    break;
                }
                let receiver = wake_receiver.clone();
                let woke = cx
                    .background_executor()
                    .spawn(async move { receiver.lock().unwrap().recv().is_ok() })
                    .await;
                if !woke {
                    wake_closed = true;
                }
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            session_service,
            pty_receiver,
            theme,
            scroll: ScrollHandle::new(),
            last_terminal_size: None,
            follow_tail: true,
            block_ui: HashMap::new(),
            last_selection: None,
            editor_layout: Rc::new(RefCell::new(InputEditorState::new())),
            editor_char_width: 1.0,
            editor_scroll_accum: 0.0,
            pinned_block: None,
            stream_closed: false,
            did_focus: false,
            _drain_task,
        }
    }

    /// Consumes chunks for at most `DRAIN_BUDGET`, returning
    /// `(changed, closed, more)`.
    fn drain_output(&mut self, started: Instant) -> (bool, bool, bool) {
        let mut changed = false;
        while started.elapsed() < DRAIN_BUDGET {
            match self.pty_receiver.try_recv() {
                Ok(chunk) => {
                    self.session_service.ingest_bytes(&chunk);
                    changed = true;
                }
                Err(TryRecvError::Empty) => return (changed, false, false),
                Err(TryRecvError::Disconnected) => {
                    if !self.stream_closed {
                        self.session_service.finalize_stream();
                        self.stream_closed = true;
                        changed = true;
                    }
                    return (changed, true, false);
                }
            }
        }
        (changed, false, true)
    }
}
