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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyText {
    text: String,
    runs: Vec<BodyRun>,
    row_starts: Vec<usize>,
}

/// A contiguous row window sliced out of a [`BodyText`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodySlice {
    text: String,
    runs: Vec<SliceRun>,
    start_byte: usize,
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
        self.row_starts.len()
    }

    /// Slices rows `start_row..end_row`, applying `selection` (absolute byte
    /// offsets into the full body) as `selected` runs.
    pub fn slice(
        &self,
        start_row: usize,
        end_row: usize,
        selection: Option<Range<usize>>,
    ) -> BodySlice {
        let start_row = start_row.min(self.row_count());
        let end_row = end_row.min(self.row_count()).max(start_row);
        if start_row >= end_row {
            return BodySlice {
                text: String::new(),
                runs: Vec::new(),
                start_byte: self.text.len(),
            };
        }

        let start_byte = self.row_starts[start_row];
        let end_byte = if end_row == self.row_count() {
            self.text.len()
        } else {
            // `row_starts[end_row]` sits after the separator before that row.
            self.row_starts[end_row] - 1
        };
        let selection = selection.map(|range| clamp_selection(range, &self.text));

        let mut runs = Vec::new();
        for run in &self.runs {
            let lo = run.range.start.max(start_byte);
            let hi = run.range.end.min(end_byte);
            if lo >= hi {
                continue;
            }
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
            text: self.text[start_byte..end_byte].to_owned(),
            runs,
            start_byte,
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

    /// Concatenated text of the selected runs; used by the body tests.
    #[cfg(test)]
    pub fn selected_text(&self) -> String {
        let mut text = String::new();
        for run in &self.runs {
            if run.selected {
                text.push_str(&self.text[run.range.clone()]);
            }
        }
        text
    }
}

/// Flattens rows into newline-separated text with one run per styled segment.
pub fn build_body_text<'a>(rows: impl IntoIterator<Item = &'a Row>) -> BodyText {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut row_starts = Vec::new();
    let mut last_row_empty = true;
    let mut rows_seen = 0usize;

    for row in rows {
        if rows_seen > 0 {
            push_newline(&mut text, &mut runs);
        }
        row_starts.push(text.len());
        last_row_empty = true;
        for span in &row.spans {
            if span.text.is_empty() {
                continue;
            }
            last_row_empty = false;
            let start = text.len();
            text.push_str(&span.text);
            merge_run(&mut runs, start..text.len(), span.style);
        }
        rows_seen += 1;
    }

    // `split('\n')` drops a trailing empty segment unless the text ends with a
    // newline; keep the final blank row addressable.
    if rows_seen > 1 && last_row_empty && !text.ends_with('\n') {
        push_newline(&mut text, &mut runs);
    }

    BodyText {
        text,
        runs,
        row_starts,
    }
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
