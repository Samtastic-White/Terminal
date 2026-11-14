//! `BlockTextWriter` — bounded `vte::Perform` implementation.
//!
//! Interprets normal-mode output: plain text, CR/LF/BS/HT, 16/256/24-bit SGR
//! colors, style attributes, `EL`/`ED`, auto-wrap at PTY width, and cursor
//! tracking sufficient for `\r` rewrites. The current logical row is exposed
//! read-only for live rendering, so prompts and typed input appear before a
//! newline commits them.
//!
//! The full terminal-grid state machine is never hand-rolled here. Everything
//! outside this subset is consumed silently: never printed, never panicked on.

mod erase;
mod osc;
mod sgr;
mod text;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use tracing::trace;
use vte::{Parser, Perform};

use crate::domain::output::{Row, TextStyle};
use crate::domain::semantic::SemanticEvent;
use crate::domain::size::TerminalSize;
use crate::domain::terminal_surface::{SurfaceItem, TerminalSurfacePort};

/// Maximum raw OSC payload retained per sequence, in bytes. Unterminated
/// sequences degrade to a truncated payload instead of growing without bound.
const OSC_MAX_BYTES: usize = 65_536;

/// The subset interpreter. Kept separate from `BlockTextWriter` so `write()`
/// can split-borrow the persistent `vte::Parser` and this state across one
/// `advance` call.
#[derive(Debug)]
struct Core {
    /// PTY width in columns. New bytes wrap here; stored rows keep their
    /// creation-time width.
    cols: usize,
    /// Active SGR style. Applies from the next printed character; no
    /// retroactive restyling.
    style: TextStyle,
    /// Cursor column of the active row. Only the current row is tracked.
    col: usize,
    /// VT100 deferred auto-wrap: set after printing at the last column;
    /// cleared by explicit cursor motion or a newline.
    pending_wrap: bool,
    /// Addressable cells of the active row; `None` means never written.
    /// Bounded by `cols` plus mid-rewrite leftovers.
    active: Vec<Option<ActiveCell>>,
    /// Committed rows and recognized semantic events awaiting transport.
    items: VecDeque<SurfaceItem>,
}

/// One addressable character of the active row plus the style it was printed
/// with. In-place rewrites overwrite cells, never spans.
///
/// `cont` marks continuation columns of a wide glyph: fold emits the character
/// once at its head cell and skips continuations, which exist only to consume
/// display columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveCell {
    ch: char,
    style: TextStyle,
    cont: bool,
}

impl Core {
    fn resize_cols(&mut self, cols: u16) {
        self.cols = usize::from(cols.max(1));
        self.active.truncate(self.cols);
        if self.col >= self.cols {
            self.col = self.cols - 1;
            self.pending_wrap = false;
        }
    }
}

/// The `vte::Perform` subset. Everything outside it is a no-op.
impl Perform for Core {
    fn print(&mut self, c: char) {
        self.print(c);
    }

    fn execute(&mut self, byte: u8) {
        self.execute_c0(byte);
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore || !intermediates.is_empty() {
            trace!(family = "csi", "parser input ignored");
            return; // Malformed or unsupported: consumed silently.
        }
        match action {
            'K' => {
                let mode = params
                    .iter()
                    .next()
                    .and_then(|group| group.first().copied())
                    .unwrap_or(0);
                self.erase_in_line(mode);
            }
            'J' => {
                let mode = params
                    .iter()
                    .next()
                    .and_then(|group| group.first().copied())
                    .unwrap_or(0);
                self.erase_in_display(mode);
            }
            'm' => self.sgr(params),
            // Cursor addressing and grid sequences stay out of this subset.
            _ => trace!(family = "csi", "parser input ignored"),
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.handle_osc(params);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, action: u8) {
        if !ignore && intermediates.is_empty() && action == b'c' {
            self.emit(SemanticEvent::TerminalReset);
        } else {
            // Charset shifts, DECALN, etc.: outside the subset, absorbed.
            trace!(family = "esc", "parser input ignored");
        }
    }
}

/// Consumes raw child bytes chunked at PTY-read boundaries (mid-sequence
/// splits included: the `vte` FSM is stateful) and yields committed styled
/// rows and semantic events in stream order.
///
/// No `Debug`: the inner `vte::Parser` does not implement it, and the writer's
/// observable state is exactly the item stream.
pub struct BlockTextWriter {
    parser: Parser<OSC_MAX_BYTES>,
    core: Core,
}

impl BlockTextWriter {
    pub fn new(cols: u16) -> Self {
        Self {
            parser: Parser::<OSC_MAX_BYTES>::new_with_size(),
            core: Core {
                cols: usize::from(cols.max(1)),
                style: TextStyle::default(),
                col: 0,
                pending_wrap: false,
                active: Vec::new(),
                items: VecDeque::new(),
            },
        }
    }

    #[cfg(test)]
    pub fn cols(&self) -> u16 {
        self.core.cols as u16
    }

    /// New bytes wrap at the new width. Already stored rows are never
    /// re-wrapped; the active row is truncated at the new width.
    #[cfg(test)]
    pub fn resize_cols(&mut self, cols: u16) {
        self.core.resize_cols(cols);
    }

    /// Consumes a raw chunk. UTF-8 multibyte sequences split across calls
    /// reassemble.
    pub fn write(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.core, bytes);
    }

    /// Drains committed rows and semantic events for transport.
    pub fn take_items(&mut self) -> Vec<SurfaceItem> {
        self.core.items.drain(..).collect()
    }

    /// Test-only view of the row stream; semantic events are discarded.
    #[cfg(test)]
    pub fn take_committed_rows(&mut self) -> Vec<Row> {
        self.take_items()
            .into_iter()
            .filter_map(|item| match item {
                SurfaceItem::Row(row) => Some(row),
                SurfaceItem::Semantic(_) => None,
            })
            .collect()
    }

    /// Non-destructive snapshot of the row still being built (the shell
    /// prompt or the line being typed), for live rendering. It never commits
    /// or clears the row; only a newline, auto-wrap, or `flush` retires it.
    pub fn active_row(&self) -> Option<Row> {
        self.core.fold_active()
    }

    /// Flushes the last partially built row. A fully unwritten row
    /// fabricates nothing.
    pub fn flush_active_row(&mut self) {
        self.core.flush_active();
    }
}

impl TerminalSurfacePort for BlockTextWriter {
    fn interpret(&mut self, bytes: &[u8]) -> Vec<SurfaceItem> {
        self.write(bytes);
        self.take_items()
    }

    fn resize(&mut self, size: TerminalSize) {
        // Direct on Core: an inherent method with the same name would recurse
        // through the port method.
        self.core.resize_cols(size.cols);
    }

    fn active_row(&self) -> Option<Row> {
        BlockTextWriter::active_row(self)
    }

    fn flush(&mut self) -> Vec<SurfaceItem> {
        self.flush_active_row();
        self.take_items()
    }
}
