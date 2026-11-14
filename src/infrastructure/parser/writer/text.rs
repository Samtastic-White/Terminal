//! Character printing, C0 control handling, and row folding for the writer.

use unicode_width::UnicodeWidthChar;

use super::{ActiveCell, Core};
use crate::domain::output::{Row, Span, TextStyle};
use crate::domain::terminal_surface::SurfaceItem;

/// HT advances to the next multiple of this stop (VT convention).
const TAB_STOP: usize = 8;

impl Core {
    /// Folds the active row into finished spans and retires it.
    ///
    /// An all-unwritten row still commits as an empty `Row`: double LF is real
    /// blank-line content. Unwritten leading and interior columns are padded
    /// as plain blanks; trailing unwritten cells are dropped. Historic cells
    /// beyond a rewrite stay: `\r` moves the cursor but does not erase.
    pub(super) fn commit(&mut self) {
        let row = match self.active.iter().rposition(|cell| cell.is_some()) {
            Some(last) => {
                let row = fold_cells(&self.active[..=last]);
                self.active.drain(..=last);
                row
            }
            None => {
                self.active.clear();
                Row { spans: Vec::new() }
            }
        };
        self.items.push_back(SurfaceItem::Row(row));
        self.pending_wrap = false;
    }

    /// Prints one character with deferred auto-wrap semantics.
    ///
    /// Zero-width characters (combining, ZWJ) are dropped. A character at the
    /// last column stays in place with `pending_wrap`; the next printable char
    /// commits the row first. A wide character that straddles the boundary
    /// commits the row before placing (xterm behavior).
    pub(super) fn print(&mut self, c: char) {
        let Some(w) = c.width() else { return };
        if w == 0 {
            return;
        }
        if w > self.cols {
            // A glyph wider than the PTY cannot be placed: degrade, never panic.
            return;
        }
        if self.pending_wrap {
            self.commit();
            self.col = 0;
        }
        if self.col + w > self.cols {
            self.commit();
            self.col = 0;
        }
        let cell_head = ActiveCell {
            ch: c,
            style: self.style,
            cont: false,
        };
        let cont = ActiveCell {
            ch: c,
            style: self.style,
            cont: true,
        };
        let start = self.col;
        let end = start + w;
        if self.active.len() < end {
            self.active.resize(end, None);
        }
        if w == 1 {
            // Never leave half a wide glyph behind: clear the orphaned mate.
            if let Some(cell) = self.active.get(start).copied().flatten()
                && cell.cont
                && start > 0
            {
                self.active[start - 1] = None;
            }
            if let Some(cell) = self.active.get(start + 1).copied().flatten()
                && cell.cont
            {
                self.active[start + 1] = None;
            }
            self.active[start] = Some(cell_head);
        } else {
            self.active[start] = Some(cell_head);
            for slot in &mut self.active[(start + 1)..end] {
                *slot = Some(cont);
            }
        }
        self.col = end;
        if self.col == self.cols {
            self.pending_wrap = true;
        }
    }

    /// C0 controls of the subset: CR, LF/VT/FF, BS, HT. BEL and every other
    /// C0 is consumed silently.
    pub(super) fn execute_c0(&mut self, byte: u8) {
        match byte {
            // Carriage return: column 0, the progress-bar engine.
            b'\r' => {
                self.col = 0;
                self.pending_wrap = false;
            }
            // LF (and VT/FF): commit the row, keep the column. A bare `\n`
            // staircases; shells emit CRLF.
            b'\n' | 0x0B | 0x0C => {
                self.commit();
                self.col = self.col.min(self.cols.saturating_sub(1));
            }
            // Backspace: one column left, floor 0.
            0x08 => {
                self.pending_wrap = false;
                self.col = self.col.saturating_sub(1);
            }
            // Tab: next multiple-of-8 stop, clipped at the last printable
            // column. Cursor movement never wraps by itself.
            0x09 => {
                let stop = (self.col / TAB_STOP + 1) * TAB_STOP;
                self.col = stop.min(self.cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            // BEL and every other C0: outside the subset, ignored.
            _ => {}
        }
    }

    /// Non-destructive snapshot of the written prefix of the active row.
    /// `None` when no cell has been written. Uses the same folding rules as
    /// `commit`, so the live row and the row committed on LF are identical.
    pub(super) fn fold_active(&self) -> Option<Row> {
        let last = self.active.iter().rposition(|cell| cell.is_some())?;
        Some(fold_cells(&self.active[..=last]))
    }
}

/// Folds addressable cells into styled spans. Continuation columns of wide
/// glyphs are skipped (the head cell already emitted the character); unwritten
/// cells inside the written prefix become plain blanks.
fn fold_cells(cells: &[Option<ActiveCell>]) -> Row {
    let mut spans: Vec<Span> = Vec::new();
    let mut run_style: Option<TextStyle> = None;
    let mut run_text = String::new();

    for cell in cells {
        let cell = cell.unwrap_or(ActiveCell {
            ch: ' ',
            style: TextStyle::default(),
            cont: false,
        });
        // Continuation columns were already emitted by their head cell.
        if cell.cont {
            continue;
        }
        if run_style != Some(cell.style) {
            if let Some(style) = run_style.take() {
                spans.push(Span {
                    style,
                    text: std::mem::take(&mut run_text),
                });
            }
            run_style = Some(cell.style);
            run_text.reserve(16);
        }
        run_text.push(cell.ch);
    }
    if let Some(style) = run_style.take() {
        spans.push(Span {
            style,
            text: run_text,
        });
    }

    Row { spans }
}
