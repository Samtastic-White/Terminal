//! `EL`/`ED` erase operations over the active logical row.

use super::{ActiveCell, Core};
use crate::domain::output::{Color, TextStyle};

impl Core {
    /// Erases a cell range painting with the background of the active style
    /// at erase time. `to_inclusive` values clamp at the last written cell;
    /// erasing past the cursor is a no-op and never erases backwards.
    fn erase(&mut self, from: usize, to_inclusive: usize) {
        let Some(last) = self.active.iter().rposition(|cell| cell.is_some()) else {
            return;
        };
        if from > last {
            return;
        }
        let to = to_inclusive.min(last);
        if from > to {
            return;
        }
        let blank = ActiveCell {
            ch: ' ',
            style: erase_style(self.style),
            cont: false,
        };
        for slot in &mut self.active[from..=to] {
            *slot = Some(blank);
        }
    }

    /// `CSI K` — erase in the current logical row.
    pub(super) fn erase_in_line(&mut self, mode: u16) {
        let col = self.col.min(self.active.len().saturating_sub(1));
        match mode {
            // From cursor to end.
            0 => self.erase(col, usize::MAX),
            // From start through the cursor inclusive.
            1 => self.erase(0, col),
            // Whole row.
            2 => self.erase(0, usize::MAX),
            _ => {}
        }
    }

    /// `CSI J` — erases only the current logical row; this writer has no grid,
    /// so regions above and below are not materialized. ED 3 (scrollback
    /// clear) is ignored.
    pub(super) fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => self.erase_in_line(0),
            1 => self.erase_in_line(1),
            2 => self.erase_in_line(2),
            _ => {}
        }
    }
}

/// Blank style used by `EL`/`ED`: paints with the active background only.
fn erase_style(style: TextStyle) -> TextStyle {
    TextStyle {
        fg: Color::Default,
        bg: style.bg,
        bold: false,
        italic: false,
        underline: false,
    }
}
