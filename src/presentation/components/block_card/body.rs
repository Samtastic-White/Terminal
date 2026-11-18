//! Flat body text: rows flattened into one string with styled runs, sliced
//! into visible windows, with selection clamping at UTF-8 boundaries.

use std::ops::Range;

use crate::domain::output::{Row, TextStyle as OutputStyle};

/// A styled byte range of the flat block body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyRun {
    range: Range<usize>,
    style: OutputStyle,
}

#[cfg(test)]
impl BodyRun {
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    pub fn style(&self) -> OutputStyle {
        self.style
    }
}

/// Rows flattened into one newline-separated string plus styled runs.
///
/// `row_starts[i]` is the byte offset where row `i` begins. A trailing blank
/// row needs an extra `\n` so `TextLayout`'s newline-based indexing keeps it.
///
/// The body grows incrementally: [`BodyText::push_row`] appends without
/// touching the stored prefix, and [`BodyText::drop_front`] only advances a
/// base offset. Evicted bytes are reclaimed by an amortized compaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyText {
    text: String,
    runs: Vec<BodyRun>,
    row_starts: Vec<usize>,
    /// Index into `row_starts` of the first live row.
    first_row: usize,
    /// Byte offset of the first live row inside `text`.
    base_byte: usize,
}

/// A contiguous row window sliced out of a [`BodyText`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodySlice {
    text: String,
    runs: Vec<SliceRun>,
    start_byte: usize,
    /// Committed rows in the slice, even when their text is empty.
    rows: usize,
}

/// One styled segment of a [`BodySlice`], already localized to the slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceRun {
    range: Range<usize>,
    style: OutputStyle,
    selected: bool,
}

impl SliceRun {
    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    pub fn style(&self) -> OutputStyle {
        self.style
    }

    pub fn selected(&self) -> bool {
        self.selected
    }
}

impl BodyText {
    #[cfg(test)]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub fn runs(&self) -> &[BodyRun] {
        &self.runs
    }

    pub fn row_count(&self) -> usize {
        self.row_starts.len() - self.first_row
    }

    /// Plain text for an absolute selection over the live body. The slice is
    /// bounded to the rows the selection touches, so copying never duplicates
    /// the whole block.
    pub fn selected_text(&self, range: Range<usize>) -> String {
        let live = &self.text[self.base_byte..];
        let range = clamp_selection(range, live);
        if range.is_empty() {
            return String::new();
        }

        let rows = &self.row_starts[self.first_row..];
        let start = self.base_byte + range.start;
        let end = self.base_byte + range.end;
        let start_row = rows
            .partition_point(|row_start| *row_start <= start)
            .saturating_sub(1);
        let end_row = rows.partition_point(|row_start| *row_start <= end);
        self.slice(start_row, end_row.max(start_row + 1), Some(range))
            .selected_text()
    }

    /// Appends one row. Equivalent to one step of [`build_body_text`].
    pub fn push_row(&mut self, row: &Row) {
        if self.row_count() > 0 {
            push_newline(&mut self.text, &mut self.runs);
        }
        self.row_starts.push(self.text.len());
        for span in &row.spans {
            if span.text.is_empty() {
                continue;
            }
            let start = self.text.len();
            self.text.push_str(&span.text);
            merge_run(&mut self.runs, start..self.text.len(), span.style);
        }
    }

    /// Drops the oldest `count` rows without copying the live tail; dead bytes
    /// are reclaimed once they dominate the buffer.
    pub fn drop_front(&mut self, count: usize) {
        let count = count.min(self.row_count());
        if count == 0 {
            return;
        }
        self.first_row += count;
        self.base_byte = self.row_starts[self.first_row];
        if self.base_byte >= 64 * 1024 && self.base_byte * 2 >= self.text.len() {
            self.compact();
        }
    }

    fn compact(&mut self) {
        let base = self.base_byte;
        self.text.drain(..base);
        self.runs.retain_mut(|run| {
            if run.range.end <= base {
                return false;
            }
            run.range.start = run.range.start.saturating_sub(base);
            run.range.end -= base;
            true
        });
        self.row_starts.drain(..self.first_row);
        for start in &mut self.row_starts {
            *start -= base;
        }
        self.first_row = 0;
        self.base_byte = 0;
    }

    /// Slices rows `start_row..end_row`, applying `selection` (absolute byte
    /// offsets into the full body) as `selected` runs.
    pub fn slice(
        &self,
        start_row: usize,
        end_row: usize,
        selection: Option<Range<usize>>,
    ) -> BodySlice {
        let row_count = self.row_count();
        let start_row = start_row.min(row_count);
        let end_row = end_row.min(row_count).max(start_row);
        let body_len = self.text.len() - self.base_byte;
        if start_row >= end_row {
            return BodySlice {
                text: String::new(),
                runs: Vec::new(),
                start_byte: body_len,
                rows: 0,
            };
        }

        let start_byte = self.row_starts[self.first_row + start_row] - self.base_byte;
        let end_byte = if end_row == row_count {
            body_len
        } else {
            // `row_starts[end_row]` sits after the separator before that row.
            self.row_starts[self.first_row + end_row] - self.base_byte - 1
        };
        let selection = selection.map(|range| clamp_selection(range, &self.text[self.base_byte..]));

        let mut runs = Vec::new();
        for run in &self.runs {
            let lo = run.range.start.max(start_byte + self.base_byte);
            let hi = run.range.end.min(end_byte + self.base_byte);
            if lo >= hi {
                continue;
            }
            let lo = lo - self.base_byte;
            let hi = hi - self.base_byte;
            match &selection {
                Some(selected) if selected.start < hi && selected.end > lo => {
                    let sel_lo = lo.max(selected.start);
                    let sel_hi = hi.min(selected.end);
                    push_slice_run(&mut runs, lo..sel_lo, start_byte, run.style, false);
                    push_slice_run(&mut runs, sel_lo..sel_hi, start_byte, run.style, true);
                    push_slice_run(&mut runs, sel_hi..hi, start_byte, run.style, false);
                }
                _ => push_slice_run(&mut runs, lo..hi, start_byte, run.style, false),
            }
        }

        BodySlice {
            text: self.text[self.base_byte + start_byte..self.base_byte + end_byte].to_owned(),
            runs,
            start_byte,
            rows: end_row - start_row,
        }
    }
}

impl BodySlice {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn runs(&self) -> &[SliceRun] {
        &self.runs
    }

    pub fn start_byte(&self) -> usize {
        self.start_byte
    }

    /// Concatenated text of the selected runs.
    pub fn selected_text(&self) -> String {
        let mut text = String::new();
        for run in &self.runs {
            if run.selected {
                text.push_str(&self.text[run.range.clone()]);
            }
        }
        text
    }

    /// Appends the in-progress row after the committed slice. Selection ranges
    /// were computed before this call, so live text is never selected.
    pub fn push_live_row(&mut self, row: &Row) {
        if self.rows > 0 {
            let start = self.text.len();
            self.text.push('\n');
            push_slice_run(
                &mut self.runs,
                start..start + 1,
                0,
                OutputStyle::default(),
                false,
            );
        }
        self.rows += 1;
        for span in &row.spans {
            if span.text.is_empty() {
                continue;
            }
            let start = self.text.len();
            self.text.push_str(&span.text);
            push_slice_run(&mut self.runs, start..self.text.len(), 0, span.style, false);
        }
    }
}

/// Flattens rows into newline-separated text with one run per styled segment.
pub fn build_body_text<'a>(rows: impl IntoIterator<Item = &'a Row>) -> BodyText {
    let mut body = BodyText::default();
    for row in rows {
        body.push_row(row);
    }
    body
}

/// Clamps a byte range to character boundaries so slicing never panics on
/// multi-byte glyphs.
pub fn clamp_selection(range: Range<usize>, text: &str) -> Range<usize> {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    start..end
}

fn push_newline(text: &mut String, runs: &mut Vec<BodyRun>) {
    let start = text.len();
    text.push('\n');
    merge_run(runs, start..text.len(), OutputStyle::default());
}

fn merge_run(runs: &mut Vec<BodyRun>, range: Range<usize>, style: OutputStyle) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut()
        && last.style == style
        && last.range.end == range.start
    {
        last.range.end = range.end;
        return;
    }
    runs.push(BodyRun { range, style });
}

fn push_slice_run(
    runs: &mut Vec<SliceRun>,
    range: Range<usize>,
    slice_start: usize,
    style: OutputStyle,
    selected: bool,
) {
    if range.is_empty() {
        return;
    }
    let local = range.start - slice_start..range.end - slice_start;
    if let Some(last) = runs.last_mut()
        && last.style == style
        && last.selected == selected
        && last.range.end == local.start
    {
        last.range.end = local.end;
        return;
    }
    runs.push(SliceRun {
        range: local,
        style,
        selected,
    });
}
