//! Bounded command editor with a pure, cell-based display map.
//!
//! The editor owns the command buffer as UTF-8 text with LF line breaks, a
//! caret on a char boundary, and an anchor/focus selection. Wrapping, tabs and
//! character widths are expressed in display cells, never pixels, so every
//! editing and hit-testing rule stays headless-testable.

// The session orchestration consumes this model; until that wiring exists the
// unit tests are the only users in a non-test build.
#![cfg_attr(not(test), allow(dead_code))]

use std::cell::RefCell;
use std::ops::Range;

use unicode_width::UnicodeWidthChar;

/// Rows a presentation dock may allocate for the editor; the dock also
/// clamps this value to the window height so the block list keeps rows.
pub const MAX_VISIBLE_LINES: usize = 28;
/// Display cells one literal tab advances to its next stop.
pub const TAB_CELLS: usize = 4;

/// Hard caps for the command buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorLimits {
    pub max_bytes: usize,
    pub max_lines: usize,
}

impl Default for EditorLimits {
    fn default() -> Self {
        Self {
            max_bytes: 65_536,
            max_lines: 512,
        }
    }
}

/// Vertical direction of caret, page and scroll motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

/// Result of a typed insertion; typing at the cap is ignored atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertOutcome {
    pub inserted: bool,
}

/// Result of a paste insertion, including what did not fit under the caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteOutcome {
    pub inserted: bool,
    pub dropped_bytes: usize,
    pub dropped_lines: usize,
}

/// A run of source characters that renders as one visual line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    pub source_start: usize,
    pub source_end: usize,
    pub cells: usize,
    pub wrapped: bool,
}

/// One visual line handed to presentation: display text, no source bytes.
///
/// Cell ranges are the geometric contract; the `*_offset` fields are byte
/// offsets into the tab-expanded `text`, which is what the shaper needs to
/// place the caret and selection quads without accumulating cell-to-pixel
/// error on wide glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorLine {
    pub text: String,
    pub selected_cells: Option<Range<usize>>,
    pub caret_cell: Option<usize>,
    pub caret_offset: Option<usize>,
    pub selected_offsets: Option<Range<usize>>,
    /// The selection includes this line's break, so its highlight extends one
    /// cell past the text.
    pub newline_selected: bool,
}

/// The visible window of the editor; at most `visible_lines` short lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditorView {
    pub lines: Vec<EditorLine>,
    pub first_line: usize,
    pub total_lines: usize,
    pub caret: (usize, usize),
    pub has_selection: bool,
}

/// Wrapping, tab and width resolution for one text and one column count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayMap {
    text: String,
    lines: Vec<VisualLine>,
}

impl DisplayMap {
    pub fn new(text: &str, cols: usize) -> Self {
        let cols = cols.max(1);
        let mut lines = Vec::new();
        let mut start = 0;
        for (index, _) in text.match_indices('\n') {
            push_visual_lines(&mut lines, text, start, index, cols);
            start = index + 1;
        }
        push_visual_lines(&mut lines, text, start, text.len(), cols);
        Self {
            text: text.to_owned(),
            lines,
        }
    }

    pub fn lines(&self) -> &[VisualLine] {
        &self.lines
    }

    /// Visual line containing a caret byte; a soft-wrap boundary belongs to the
    /// line that starts there, a hard newline to the line that ends there.
    pub fn line_of(&self, byte: usize) -> usize {
        for (index, line) in self.lines.iter().enumerate() {
            if byte < line.source_start {
                break;
            }
            if byte < line.source_end {
                return index;
            }
            if byte == line.source_end {
                let soft_wrap = self
                    .lines
                    .get(index + 1)
                    .is_some_and(|next| next.wrapped && next.source_start == byte);
                return if soft_wrap { index + 1 } else { index };
            }
        }
        self.lines.len().saturating_sub(1)
    }

    /// Display cell of a caret byte inside its visual line.
    pub fn cell_of(&self, byte: usize) -> usize {
        let line = &self.lines[self.line_of(byte)];
        let end = byte.clamp(line.source_start, line.source_end);
        cells_until(&self.text, line.source_start, end)
    }

    /// Nearest char boundary to a cell inside a visual line; cells past the
    /// line end land on its last boundary.
    pub fn byte_at(&self, line: usize, cell: usize) -> usize {
        let Some(line) = self.lines.get(line) else {
            return self.text.len();
        };
        let mut cells = 0;
        for (offset, ch) in self.text[line.source_start..line.source_end].char_indices() {
            let byte = line.source_start + offset;
            let advance = cell_advance(ch, cells);
            if cell < cells + advance {
                if ch == '\t' {
                    return byte;
                }
                let half = advance.div_ceil(2);
                return if cell < cells + half {
                    byte
                } else {
                    byte + ch.len_utf8()
                };
            }
            cells += advance;
        }
        line.source_end
    }
}

fn push_visual_lines(
    lines: &mut Vec<VisualLine>,
    text: &str,
    start: usize,
    end: usize,
    cols: usize,
) {
    let mut segment_start = start;
    let mut cells = 0;
    let mut wrapped = false;
    for (offset, ch) in text[start..end].char_indices() {
        let byte = start + offset;
        let advance = cell_advance(ch, cells);
        if cells + advance > cols && cells > 0 {
            lines.push(VisualLine {
                source_start: segment_start,
                source_end: byte,
                cells,
                wrapped,
            });
            segment_start = byte;
            cells = cell_advance(ch, 0);
            wrapped = true;
        } else {
            cells += advance;
        }
    }
    lines.push(VisualLine {
        source_start: segment_start,
        source_end: end,
        cells,
        wrapped,
    });
}

fn cell_advance(ch: char, cells: usize) -> usize {
    if ch == '\t' {
        TAB_CELLS - cells % TAB_CELLS
    } else {
        char_cells(ch)
    }
}

fn char_cells(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn cells_until(text: &str, start: usize, end: usize) -> usize {
    let mut cells = 0;
    for ch in text[start..end].chars() {
        cells += cell_advance(ch, cells);
    }
    cells
}

fn newline_count(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
}

/// Normalizes pasted text: `CRLF`/`CR` become `LF`, and every other control
/// character is dropped so no terminal escape can enter the buffer.
pub fn sanitize_paste(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                sanitized.push('\n');
            }
            '\n' | '\t' => sanitized.push(ch),
            ch if ch.is_control() => {}
            ch => sanitized.push(ch),
        }
    }
    sanitized
}

#[derive(Debug)]
struct CachedMap {
    revision: u64,
    cols: usize,
    map: DisplayMap,
}

/// Mutable command buffer plus its cached visual layout.
#[derive(Debug)]
pub struct Editor {
    text: String,
    caret: usize,
    anchor: Option<usize>,
    desired_cell: Option<usize>,
    wrap_cols: usize,
    first_line: usize,
    visible_lines: usize,
    limits: EditorLimits,
    revision: u64,
    cache: RefCell<Option<CachedMap>>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(EditorLimits::default())
    }
}

impl Editor {
    pub fn new(limits: EditorLimits) -> Self {
        Self {
            text: String::new(),
            caret: 0,
            anchor: None,
            desired_cell: None,
            wrap_cols: 1,
            first_line: 0,
            visible_lines: 1,
            limits,
            revision: 0,
            cache: RefCell::new(None),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Monotonic counter bumped whenever the text content changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Idempotent: returns true only when the column count actually changed.
    pub fn set_wrap_cols(&mut self, cols: usize) -> bool {
        let cols = cols.max(1);
        if cols == self.wrap_cols {
            return false;
        }
        self.wrap_cols = cols;
        self.first_line = self.first_line.min(self.max_first_line());
        self.ensure_caret_visible();
        true
    }

    /// Idempotent: returns true only when the row count actually changed.
    pub fn set_visible_lines(&mut self, lines: usize) -> bool {
        let lines = lines.clamp(1, MAX_VISIBLE_LINES);
        if lines == self.visible_lines {
            return false;
        }
        self.visible_lines = lines;
        self.first_line = self.first_line.min(self.max_first_line());
        self.ensure_caret_visible();
        true
    }

    pub fn insert_text(&mut self, text: &str) -> InsertOutcome {
        let sanitized = sanitize_paste(text);
        if sanitized.is_empty() {
            return InsertOutcome { inserted: false };
        }
        let range = self.selection().unwrap_or(self.caret..self.caret);
        if !self.fits(&range, &sanitized) {
            return InsertOutcome { inserted: false };
        }
        self.apply_edit(range, &sanitized);
        InsertOutcome { inserted: true }
    }

    pub fn insert_paste(&mut self, text: &str) -> PasteOutcome {
        let sanitized = sanitize_paste(text);
        if sanitized.is_empty() {
            return PasteOutcome {
                inserted: false,
                dropped_bytes: 0,
                dropped_lines: 0,
            };
        }
        let range = self.selection().unwrap_or(self.caret..self.caret);
        let removed = range.end - range.start;
        let byte_budget = self
            .limits
            .max_bytes
            .saturating_sub(self.text.len() - removed);
        let line_budget = self.limits.max_lines.saturating_sub(
            newline_count(&self.text[..range.start]) + newline_count(&self.text[range.end..]) + 1,
        );

        let mut end = 0;
        let mut used_lines = 0;
        for (index, ch) in sanitized.char_indices() {
            let bytes = index + ch.len_utf8();
            let lines = if ch == '\n' {
                used_lines + 1
            } else {
                used_lines
            };
            if bytes > byte_budget || lines > line_budget {
                break;
            }
            used_lines = lines;
            end = bytes;
        }
        let inserted = &sanitized[..end];
        if inserted.is_empty() {
            return PasteOutcome {
                inserted: false,
                dropped_bytes: sanitized.len(),
                dropped_lines: newline_count(&sanitized),
            };
        }
        self.apply_edit(range, inserted);
        let dropped = &sanitized[end..];
        PasteOutcome {
            inserted: true,
            dropped_bytes: dropped.len(),
            dropped_lines: newline_count(dropped),
        }
    }

    pub fn delete_backward(&mut self) -> bool {
        if let Some(range) = self.selection() {
            return self.apply_edit(range, "");
        }
        if self.caret == 0 {
            return false;
        }
        let start = self.step_back(self.caret, 1);
        self.apply_edit(start..self.caret, "")
    }

    pub fn delete_forward(&mut self) -> bool {
        if let Some(range) = self.selection() {
            return self.apply_edit(range, "");
        }
        if self.caret == self.text.len() {
            return false;
        }
        let end = self.step_forward(self.caret, 1);
        self.apply_edit(self.caret..end, "")
    }

    pub fn move_horizontal(&mut self, delta: isize, extend: bool) -> bool {
        if delta == 0 {
            return false;
        }
        let caret = self.caret;
        let anchor = self.anchor;
        self.desired_cell = None;
        if !extend && let Some(range) = self.selection() {
            self.anchor = None;
            self.caret = if delta < 0 { range.start } else { range.end };
        } else {
            let target = if delta < 0 {
                self.step_back(self.caret, delta.unsigned_abs())
            } else {
                self.step_forward(self.caret, delta as usize)
            };
            self.place_caret(target, extend);
        }
        self.ensure_caret_visible();
        self.caret != caret || self.anchor != anchor
    }

    pub fn move_vertical(&mut self, direction: Direction, extend: bool) -> bool {
        let (line, cell) = self.caret_visual_position();
        let desired = self.desired_cell.unwrap_or(cell);
        self.desired_cell = Some(desired);
        let target = match direction {
            Direction::Up => line.checked_sub(1),
            Direction::Down => (line + 1 < self.visual_line_count()).then_some(line + 1),
        };
        let Some(target) = target else {
            return false;
        };
        let byte = self.with_map(|map| map.byte_at(target, desired));
        self.place_caret(byte, extend);
        self.ensure_caret_visible();
        true
    }

    pub fn move_page(&mut self, direction: Direction, extend: bool) -> bool {
        let (line, cell) = self.caret_visual_position();
        let desired = self.desired_cell.unwrap_or(cell);
        self.desired_cell = Some(desired);
        let total = self.visual_line_count();
        let page = self.visible_lines.max(1);
        let target = match direction {
            Direction::Up => line.saturating_sub(page),
            Direction::Down => (line + page).min(total - 1),
        };
        if target == line {
            return false;
        }
        let byte = self.with_map(|map| map.byte_at(target, desired));
        self.place_caret(byte, extend);
        self.ensure_caret_visible();
        true
    }

    pub fn move_home(&mut self, extend: bool) -> bool {
        let start = self.text[..self.caret]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        if self.caret == start {
            return false;
        }
        self.desired_cell = None;
        self.place_caret(start, extend);
        self.ensure_caret_visible();
        true
    }

    pub fn move_end(&mut self, extend: bool) -> bool {
        let end = self.text[self.caret..]
            .find('\n')
            .map_or(self.text.len(), |index| self.caret + index);
        if self.caret == end {
            return false;
        }
        self.desired_cell = None;
        self.place_caret(end, extend);
        self.ensure_caret_visible();
        true
    }

    pub fn move_to_cell(&mut self, line: usize, cell: usize) -> bool {
        let byte = if line >= self.visual_line_count() {
            self.text.len()
        } else {
            self.with_map(|map| map.byte_at(line, cell))
        };
        let changed = self.caret != byte || self.anchor.is_some();
        self.caret = byte;
        self.anchor = None;
        self.desired_cell = None;
        self.ensure_caret_visible();
        changed
    }

    /// Extends the selection to a cell, keeping the existing anchor. A plain
    /// click uses [`Editor::move_to_cell`] instead and clears the anchor.
    pub fn drag_to_cell(&mut self, line: usize, cell: usize) -> bool {
        let byte = if line >= self.visual_line_count() {
            self.text.len()
        } else {
            self.with_map(|map| map.byte_at(line, cell))
        };
        let anchor = self.anchor.unwrap_or(self.caret);
        let changed = self.caret != byte || self.anchor != Some(anchor);
        self.anchor = Some(anchor);
        self.caret = byte;
        self.desired_cell = None;
        self.ensure_caret_visible();
        changed
    }

    pub fn select_all(&mut self) -> bool {
        if self.text.is_empty() {
            return false;
        }
        let changed = self.anchor != Some(0) || self.caret != self.text.len();
        self.anchor = Some(0);
        self.caret = self.text.len();
        self.desired_cell = None;
        self.ensure_caret_visible();
        changed
    }

    pub fn scroll_lines(&mut self, delta: isize) -> bool {
        let target =
            (self.first_line as isize + delta).clamp(0, self.max_first_line() as isize) as usize;
        if target == self.first_line {
            return false;
        }
        self.first_line = target;
        true
    }

    pub fn visual_line_count(&self) -> usize {
        self.with_map(|map| map.lines().len())
    }

    pub fn caret_visual_position(&self) -> (usize, usize) {
        let caret = self.caret;
        self.with_map(|map| (map.line_of(caret), map.cell_of(caret)))
    }

    pub fn selection_range(&self) -> Option<Range<usize>> {
        self.selection()
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().and_then(|range| self.text.get(range))
    }

    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// Replaces the buffer and places the caret at its end; the only recall
    /// entry point, sanitizing and truncating like a paste.
    pub fn set_text(&mut self, text: &str) -> bool {
        let sanitized = sanitize_paste(text);
        let limited = self.limit_whole(&sanitized);
        let text_changed = limited != self.text;
        if text_changed {
            self.text = limited.to_owned();
            self.revision = self.revision.wrapping_add(1);
        }
        let changed = text_changed || self.caret != self.text.len() || self.anchor.is_some();
        self.caret = self.text.len();
        self.anchor = None;
        self.desired_cell = None;
        self.first_line = 0;
        self.ensure_caret_visible();
        changed
    }

    pub fn clear(&mut self) -> bool {
        self.set_text("")
    }

    /// Removes and returns the buffer for submission, dropping exactly one
    /// trailing line break so the caller writes the single terminating `CR`.
    pub fn take_text(&mut self) -> String {
        let end = self
            .text
            .strip_suffix('\n')
            .map_or(self.text.len(), str::len);
        let taken = self.text[..end].to_owned();
        self.set_text("");
        taken
    }

    /// Visible window at the current internal scroll position.
    pub fn view(&self) -> EditorView {
        self.line_view(self.first_line, self.visible_lines)
    }

    /// Visible window starting at a given visual line.
    pub fn line_view(&self, first: usize, count: usize) -> EditorView {
        let caret = self.caret_visual_position();
        let (lines, first, total) = self.with_map(|map| {
            let all = map.lines();
            let total = all.len();
            let first = first.min(total.saturating_sub(1));
            let end = first.saturating_add(count).min(total);
            let lines = all[first..end]
                .iter()
                .enumerate()
                .map(|(offset, line)| self.editor_line(line, first + offset, caret))
                .collect();
            (lines, first, total)
        });
        EditorView {
            lines,
            first_line: first,
            total_lines: total,
            caret,
            has_selection: self.has_selection(),
        }
    }

    fn editor_line(&self, line: &VisualLine, index: usize, caret: (usize, usize)) -> EditorLine {
        let mut text = String::with_capacity(line.cells);
        let mut cells = 0;
        for ch in self.text[line.source_start..line.source_end].chars() {
            if ch == '\t' {
                let spaces = cell_advance(ch, cells);
                text.extend(std::iter::repeat_n(' ', spaces));
                cells += spaces;
            } else {
                text.push(ch);
                cells += char_cells(ch);
            }
        }
        let selected_cells = self.selection().and_then(|range| {
            let start = range.start.clamp(line.source_start, line.source_end);
            let end = range.end.clamp(line.source_start, line.source_end);
            let start_cell = cells_until(&self.text, line.source_start, start);
            let mut end_cell = cells_until(&self.text, line.source_start, end);
            let newline_selected = range.start <= line.source_end
                && range.end > line.source_end
                && self.text.as_bytes().get(line.source_end) == Some(&b'\n');
            if newline_selected {
                end_cell += 1;
            }
            (start_cell < end_cell).then_some(start_cell..end_cell)
        });
        let selected_offsets = self.selection().and_then(|range| {
            let start = self.expanded_offset(line, range.start);
            let end = self.expanded_offset(line, range.end);
            (start < end).then_some(start..end)
        });
        let newline_selected = self.selection().is_some_and(|range| {
            range.start <= line.source_end
                && range.end > line.source_end
                && self.text.as_bytes().get(line.source_end) == Some(&b'\n')
        });
        EditorLine {
            text,
            selected_cells,
            caret_cell: (caret.0 == index).then_some(caret.1),
            caret_offset: (caret.0 == index).then(|| self.expanded_offset(line, self.caret)),
            selected_offsets,
            newline_selected,
        }
    }

    /// Byte offset of a source boundary inside the tab-expanded display text
    /// of one visual line. Tabs expand to spaces, so display bytes are not
    /// source bytes.
    fn expanded_offset(&self, line: &VisualLine, byte: usize) -> usize {
        let end = byte.clamp(line.source_start, line.source_end);
        let mut cells = 0;
        let mut offset = 0;
        for ch in self.text[line.source_start..end].chars() {
            if ch == '\t' {
                let spaces = cell_advance(ch, cells);
                cells += spaces;
                offset += spaces;
            } else {
                cells += char_cells(ch);
                offset += ch.len_utf8();
            }
        }
        offset
    }

    fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(if anchor < self.caret {
            anchor..self.caret
        } else {
            self.caret..anchor
        })
    }

    fn place_caret(&mut self, caret: usize, extend: bool) {
        let caret = self.clamp_boundary(caret);
        if self.text.is_empty() {
            self.caret = 0;
            self.anchor = None;
            return;
        }
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
        self.caret = caret;
    }

    fn clamp_boundary(&self, byte: usize) -> usize {
        let mut byte = byte.min(self.text.len());
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        byte
    }

    fn step_back(&self, from: usize, count: usize) -> usize {
        let mut byte = from;
        for _ in 0..count {
            match self.text[..byte].chars().next_back() {
                Some(ch) => byte -= ch.len_utf8(),
                None => break,
            }
        }
        byte
    }

    fn step_forward(&self, from: usize, count: usize) -> usize {
        let mut byte = from;
        for _ in 0..count {
            match self.text[byte..].chars().next() {
                Some(ch) => byte += ch.len_utf8(),
                None => break,
            }
        }
        byte
    }

    fn fits(&self, range: &Range<usize>, insert: &str) -> bool {
        let removed = range.end - range.start;
        if self.text.len() - removed + insert.len() > self.limits.max_bytes {
            return false;
        }
        let lines = newline_count(&self.text[..range.start])
            + newline_count(&self.text[range.end..])
            + newline_count(insert)
            + 1;
        lines <= self.limits.max_lines
    }

    fn limit_whole<'a>(&self, text: &'a str) -> &'a str {
        let mut end = text.len().min(self.limits.max_bytes);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut lines = 0;
        for (offset, ch) in text[..end].char_indices() {
            if ch == '\n' {
                lines += 1;
                if lines + 1 > self.limits.max_lines {
                    end = offset;
                    break;
                }
            }
        }
        &text[..end]
    }

    fn apply_edit(&mut self, range: Range<usize>, insert: &str) -> bool {
        if range.is_empty() && insert.is_empty() {
            return false;
        }
        self.text.replace_range(range.clone(), insert);
        self.caret = range.start + insert.len();
        self.anchor = None;
        self.desired_cell = None;
        self.revision = self.revision.wrapping_add(1);
        self.ensure_caret_visible();
        true
    }

    fn max_first_line(&self) -> usize {
        self.visual_line_count().saturating_sub(self.visible_lines)
    }

    fn ensure_caret_visible(&mut self) {
        let (line, _) = self.caret_visual_position();
        if line < self.first_line {
            self.first_line = line;
        } else if line >= self.first_line + self.visible_lines {
            self.first_line = line + 1 - self.visible_lines;
        }
        self.first_line = self.first_line.min(self.max_first_line());
    }

    fn with_map<R>(&self, f: impl FnOnce(&DisplayMap) -> R) -> R {
        let cached = self.cache.borrow_mut().take();
        let map = match cached {
            Some(cached) if cached.revision == self.revision && cached.cols == self.wrap_cols => {
                cached.map
            }
            _ => DisplayMap::new(&self.text, self.wrap_cols),
        };
        let result = f(&map);
        *self.cache.borrow_mut() = Some(CachedMap {
            revision: self.revision,
            cols: self.wrap_cols,
            map,
        });
        result
    }
}

#[cfg(test)]
mod tests;
