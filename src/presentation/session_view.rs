//! Session view — render surface, key capture, and resize hook.
//!
//! The window bootstrap and infrastructure adapters live in the composition
//! root (`src/main.rs`); this view depends only on the application service and
//! domain types. Presentation never holds a PTY handle.

mod events;
mod render;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui::*;

use crate::application::session::SessionService;
use crate::domain::block::BlockId;
use crate::domain::size::TerminalSize;
use crate::presentation::components::block_card::BlockUiState;
use crate::presentation::theme::TerminalTheme;

const POLL_INTERVAL: Duration = Duration::from_millis(8);
/// Maximum chunks parsed per wake; remaining chunks wait for the next timer
/// tick so output ingestion never monopolizes a frame.
const MAX_CHUNKS_PER_WAKE: usize = 8;

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
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = TerminalTheme::default();

        // Bounded channel consumer that never runs on the render thread.
        // `try_recv` drains what is available, then parks on the timer; every
        // batch notifies the entity so the next frame renders the new rows.
        let _drain_task = cx.spawn(async move |this, cx| {
            loop {
                let closed = this.update(cx, |this, cx| {
                    let (changed, closed) = this.drain_output();
                    if changed {
                        cx.notify();
                    }
                    closed
                });
                match closed {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) => break, // view released
                }
                cx.background_executor().timer(POLL_INTERVAL).await;
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
            stream_closed: false,
            did_focus: false,
            _drain_task,
        }
    }

    /// Consumes every chunk currently buffered. Returns `(changed, closed)`:
    /// `changed` means the session state grew; `closed` means the reader
    /// channel disconnected and the final partial row was flushed.
    fn drain_output(&mut self) -> (bool, bool) {
        let mut changed = false;
        let mut drained = 0;
        while drained < MAX_CHUNKS_PER_WAKE {
            match self.pty_receiver.try_recv() {
                Ok(chunk) => {
                    self.session_service.ingest_bytes(&chunk);
                    changed = true;
                    drained += 1;
                }
                Err(TryRecvError::Empty) => return (changed, false),
                Err(TryRecvError::Disconnected) => {
                    if !self.stream_closed {
                        self.session_service.finalize_stream();
                        self.stream_closed = true;
                        changed = true;
                    }
                    return (changed, true);
                }
            }
        }
        (changed, false)
    }
}
