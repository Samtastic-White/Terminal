//! `src/infrastructure/parser/` — `BlockTextWriter`.
//!
//! A bounded `vte::Perform` implementation ("block text writer") that
//! interprets normal-mode output: plain text, CR/LF/BS/HT, 16-/256-/24-bit
//! SGR colors, style attributes, `EL`/`ED`, auto-wrap at PTY width, and
//! cursor tracking sufficient for `\r` rewrites (spec
//! `knowledge/specs/pty-raw-output/spec.md` BR-5; design
//! `knowledge/specs/pty-raw-output/design.md` § writer/data model).
//!
//! Scope boundary (roadmap Decision 2026-11-07, dual engine): the full
//! terminal-grid state machine is NEVER hand-rolled here. Phase 4 adopts a
//! proven crate behind a Domain port and plugs into
//! `BlockTextWriter::resize_cols`. Everything outside the subset is
//! consumed silently: never printed, never panicked on (BR-6).

use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;
use vte::{Parser, Perform};

use crate::domain::interpreter::TextInterpreterPort;
use crate::domain::output::{Color, Row, Span, TextStyle};

/// HT advances to the next multiple of this stop (design § 4; VT
/// convention).
const TAB_STOP: usize = 8;

/// The subset interpreter proper. Kept separate from `BlockTextWriter` so
/// `write()` can split-borrow the persist `vte::Parser` and this state
/// across one `advance` call.
#[derive(Debug)]
struct Core {
    /// PTY width in columns. New bytes wrap here; stored rows keep their
    /// creation-time width (BR-7).
    cols: usize,
    /// Active SGR style; applies from the next printed character on (the
    /// SGR change is instantaneous: no retroactive restyling).
    style: TextStyle,
    /// Cursor column of the active row. Only the current row is tracked
    /// (BR-5 cursor subset).
    col: usize,
    /// VT100 deferred auto-wrap: set after printing at the last column;
    /// cleared by any explicit cursor motion (CR/BS/HT) or a newline.
    pending_wrap: bool,
    /// Addressable cells of the active row; `None` = never written.
    /// Bounded by `cols` — plus mid-rewrite leftovers.
    active: Vec<Option<ActiveCell>>,
    /// Completed rows awaiting transport to the session document.
    committed: VecDeque<Row>,
}

/// One addressable character of the active row plus the style it was
/// printed with (in-place rewrites overwrite cells, never spans).
///
/// `cont` marks continuing columns of a wide (2-column) glyph: fold
/// emits the character once (its head cell) and skips continuations —
/// the second slot exists only to consume display columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveCell {
    ch: char,
    style: TextStyle,
    cont: bool,
}

impl Core {
    /// Fold the active row into finished spans and retire it.
    ///
    /// An all-unwritten row still commits (as an empty `Row`): double LF
    /// is real blank-line content. Unwritten interior/leading columns are
    /// padded as plain blanks — they are the positions the terminal
    /// displayed empty (staircase LF, tab stops); trailing unwritten
    /// cells beyond the last write are dropped. Historic cells beyond a
    /// rewrite stay — `\r` moves the cursor but does not erase (spec
    /// pin).
    fn commit(&mut self) {
        let last_written = self.active.iter().rposition(|cell| cell.is_some());
        let Some(last) = last_written else {
            self.active.clear();
            self.committed.push_back(Row { spans: Vec::new() });
            self.pending_wrap = false;
            return;
        };

        let mut spans: Vec<Span> = Vec::new();
        let mut run_style: Option<TextStyle> = None;
        let mut run_text = String::new();

        for cell in self.active.drain(..=last) {
            // Padding unwritten columns: only up to the last written one.
            let cell = cell.unwrap_or(ActiveCell {
                ch: ' ',
                style: TextStyle::default(),
                cont: false,
            });
            // Continuation column of a wide glyph: the character was
            // already emitted by its head.
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

        self.committed.push_back(Row { spans });
        self.pending_wrap = false;
    }

    /// Print one character with deferred auto-wrap semantics.
    ///
    /// - zero-width characters (combining/ZWJ) are dropped: a documented
    ///   non-goal of this phase (spec edge cases);
    /// - a character at the last column stays in place — `pending_wrap`,
    ///   the next printable char commits the row first;
    /// - a wide character that straddles the boundary commits the row
    ///   before placing (xterm behavior).
    fn print(&mut self, c: char) {
        let Some(w) = c.width() else { return };
        if w == 0 {
            return;
        }
        if w > self.cols {
            // A single glyph wider than the whole PTY cannot be placed:
            // deterministic degrade (BR-6), never a panic.
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
            // Overwriting part of a wide glyph leaves no half-glyph
            // behind: clear the orphaned mate (degenerate overlay cases
            // still degrade deterministically, spec BR-6).
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

    /// Erase a cell range painting with the background of the active
    /// style at erase time (spec edge cases). Open-ended `to_inclusive`
    /// values clamp at the last written cell — erasing past the cursor
    /// (nothing there) is a no-op, and never erases backwards.
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
    fn erase_in_line(&mut self, mode: u16) {
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

    /// `CSI J` — erase on the current logical row only (this writer has
    /// no grid: regions above/below are not materialized; ED 3
    /// scrollback-clear is ignored).
    fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => self.erase_in_line(0),
            1 => self.erase_in_line(1),
            2 => self.erase_in_line(2),
            _ => {}
        }
    }

    /// SGR dispatch with full `38;5;n` / `38;2;r;g;b` (and bg twins)
    /// parameter traversal; incomplete color sequences are consumed
    /// without altering state (BR-6).
    fn sgr(&mut self, params: &vte::Params) {
        let flat: Vec<u16> = params.iter().flatten().copied().collect();
        if flat.is_empty() {
            self.style = TextStyle::default();
            return;
        }
        let mut i = 0;
        while i < flat.len() {
            let consumed = self.apply_sgr(&flat[i..]);
            if consumed == 0 {
                break;
            }
            i += consumed;
        }
    }

    /// Apply one SGR parameter, returning how many flat pieces it
    /// consumed (color forms eat their arguments).
    fn apply_sgr(&mut self, rest: &[u16]) -> usize {
        match rest[0] {
            0 => {
                self.style = TextStyle::default();
                1
            }
            1 => {
                self.style.bold = true;
                1
            }
            3 => {
                self.style.italic = true;
                1
            }
            4 => {
                self.style.underline = true;
                1
            }
            22 => {
                self.style.bold = false;
                1
            }
            23 => {
                self.style.italic = false;
                1
            }
            24 => {
                self.style.underline = false;
                1
            }
            38 | 48 => {
                let which = if rest[0] == 38 { "fg" } else { "bg" };
                match rest.get(1) {
                    Some(&5) => match rest.get(2) {
                        Some(&idx) => {
                            self.style.set_color(which, Color::Indexed(idx as u8));
                            3
                        }
                        // Incomplete color form: the whole sequence was
                        // malformed — consume its remainder, no state
                        // change (BR-6).
                        None => rest.len(),
                    },
                    Some(&2) => match (rest.get(2), rest.get(3), rest.get(4)) {
                        (Some(&r), Some(&g), Some(&b)) => {
                            self.style.set_color(
                                which,
                                Color::Rgb {
                                    r: r as u8,
                                    g: g as u8,
                                    b: b as u8,
                                },
                            );
                            5
                        }
                        // Incomplete RGB form: consume the remainder.
                        _ => rest.len(),
                    },
                    // Malformed (missing/wrong mode): the whole sequence
                    // was broken — consume the remainder, state unchanged
                    // (BR-6).
                    _ => rest.len(),
                }
            }
            30..=37 => {
                self.style.fg = Color::Indexed((rest[0] - 30) as u8);
                1
            }
            90..=97 => {
                self.style.fg = Color::Indexed((rest[0] - 90 + 8) as u8);
                1
            }
            39 => {
                self.style.fg = Color::Default;
                1
            }
            40..=47 => {
                self.style.bg = Color::Indexed((rest[0] - 40) as u8);
                1
            }
            100..=107 => {
                self.style.bg = Color::Indexed((rest[0] - 100 + 8) as u8);
                1
            }
            49 => {
                self.style.bg = Color::Default;
                1
            }
            // Everything else (blink, reverse, fonts, underline styles…)
            // is outside the subset: ignored (BR-5).
            _ => 1,
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

impl TextStyle {
    fn set_color(&mut self, which: &str, color: Color) {
        if which == "fg" {
            self.fg = color;
        } else {
            self.bg = color;
        }
    }
}

/// The `vte::Perform` subset. Everything not named in BR-5 lands in a
/// no-op with a pointer at its Phase 4 owner (design § 10).
impl Perform for Core {
    fn print(&mut self, c: char) {
        self.print(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // CR: carriage return, column 0 — the progress-bar engine.
            b'\r' => {
                self.col = 0;
                self.pending_wrap = false;
            }
            // LF (and VT/FF behaving like LF): commit the row, keep the
            // column (a bare `\n` staircases by design; shells emit CRLF).
            b'\n' | 0x0B | 0x0C => {
                self.commit();
                self.col = self.col.min(self.cols.saturating_sub(1));
            }
            // BS: one column left, floor 0.
            0x08 => {
                self.pending_wrap = false;
                self.col = self.col.saturating_sub(1);
            }
            // HT: snap to the next multiple-of-8 stop, clipped at the
            // last printable column (cursor movement never triggers a
            // wrap by itself).
            0x09 => {
                let stop = (self.col / TAB_STOP + 1) * TAB_STOP;
                self.col = stop.min(self.cols.saturating_sub(1));
                self.pending_wrap = false;
            }
            // BEL and every other C0: outside the subset, ignored (BR-5).
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore || !intermediates.is_empty() {
            return; // Malformed/unsupported: consumed silently (BR-6).
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
            // CUP/CUC/CUD/CUU/SU/DECSET…: Phase 4 grid engine (design § 10).
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC (including future 133 markers) absorbed silently (BR-5).
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _action: u8) {
        // Charset shifts, DECALN, etc.: outside the subset, absorbed.
    }
}

/// Public writer: consumes raw child bytes (chunked at PTY-read
/// boundaries — mid-sequence splits included: the `vte` FSM is
/// stateful) and yields committed styled rows.
///
/// No `Debug`: the inner `vte::Parser` does not implement it and the
/// writer's observable state is exactly the row stream.
pub struct BlockTextWriter {
    parser: Parser,
    core: Core,
}

impl BlockTextWriter {
    /// Builds the writer for a PTY width of `cols` columns.
    pub fn new(cols: u16) -> Self {
        Self {
            parser: Parser::new(),
            core: Core {
                cols: usize::from(cols.max(1)),
                style: TextStyle::default(),
                col: 0,
                pending_wrap: false,
                active: Vec::new(),
                committed: VecDeque::new(),
            },
        }
    }

    /// Current wrap width (PTY cols).
    #[allow(dead_code)]
    pub fn cols(&self) -> u16 {
        self.core.cols as u16
    }

    /// Resize path (spec BR-7): new bytes wrap at the new width. Already
    /// stored rows are NOT re-wrapped (no reflow this phase); the active
    /// row keeps its content truncated at the new width.
    #[allow(dead_code)]
    pub fn resize_cols(&mut self, cols: u16) {
        self.core.resize_cols(cols);
    }

    /// Consume a raw chunk. Designed for the PTY reader's bounded chunks:
    /// UTF-8 multibyte sequences split across calls reassemble.
    pub fn write(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.core, bytes);
    }

    /// Drain completed rows for transport (the caller hands them to the
    /// Application session document, which enforces the line cap).
    pub fn take_committed_rows(&mut self) -> Vec<Row> {
        self.core.committed.drain(..).collect()
    }

    /// EOF finalization (spec BR-9): flushes the last partially built
    /// row, but fabricates nothing for a fully unwritten row.
    pub fn flush_active_row(&mut self) {
        if self.core.active.iter().any(|cell| cell.is_some()) {
            self.core.commit();
        }
        self.core.pending_wrap = false;
    }
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

impl TextInterpreterPort for BlockTextWriter {
    fn interpret(&mut self, bytes: &[u8]) -> Vec<Row> {
        self.write(bytes);
        self.take_committed_rows()
    }

    fn resize_cols(&mut self, cols: u16) {
        // Direct on Core: an inherent self-call with the same name would
        // recurse through the port method (they share the name by
        // contract design).
        self.core.resize_cols(cols);
    }

    fn flush(&mut self) -> Vec<Row> {
        self.flush_active_row();
        self.take_committed_rows()
    }
}

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed chunks, finalize (EOF flush), collect rows.
    fn rows_of(cols: u16, chunks: &[&[u8]]) -> Vec<Row> {
        let mut writer = BlockTextWriter::new(cols);
        for chunk in chunks {
            writer.write(chunk);
        }
        writer.flush_active_row();
        writer.take_committed_rows()
    }

    fn text(row: &Row) -> String {
        row.spans.iter().map(|span| span.text.as_str()).collect()
    }

    // ----- T2: text, C0 controls, pending-wrap -----

    #[test]
    fn plain_text_single_row() {
        let rows = rows_of(80, &[b"hello"]);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "hello");
        assert!(rows[0].spans[0].style.is_plain());
        assert!(rows[0].spans[0].text == "hello");
    }

    #[test]
    fn crlf_breaks_rows() {
        let rows = rows_of(80, &[b"a\r\nb"]);
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn bare_lf_keeps_column_and_blank_row_is_content() {
        let rows = rows_of(10, &[b"a\n\nb"]);
        // Double LF is real blank-line content between rows; the LF keeps
        // the column, so 'b' lands at column 1 (staircase by design).
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a", "", " b"]);
    }

    #[test]
    fn bs_clamps_at_floor_zero() {
        let rows = rows_of(10, &[b"\x08a\x08"]);
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn ht_snaps_to_tab_stop_and_keeps_position() {
        let rows = rows_of(16, &[b"\tab"]);
        // Tab stops at column 8: the unwritten prefix is rendered as the
        // blank cells it occupies (leading padding).
        assert_eq!(text(&rows[0]), "        ab");
    }

    #[test]
    fn ht_clamps_at_last_column_without_fabricating_rows() {
        let rows = rows_of(5, &[b"\t\t\t\tX"]);
        assert_eq!(rows.len(), 1);
        assert_eq!(text(&rows[0]), "    X");
    }

    #[test]
    fn deferred_wrap_at_last_column_commits_once() {
        let rows = rows_of(5, &[b"abcde", b"x"]);
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["abcde", "x"]);
    }

    #[test]
    fn trailing_lf_never_fabricates_rows() {
        let rows = rows_of(10, &[b"a\n"]);
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn cr_rewrites_overwrite_in_place() {
        let rows = rows_of(10, &[b"hello\rX\n"]);
        // \r moves the cursor but does not erase: X overwrites 'h'.
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["Xello"]);
    }

    // ----- T3: SGR colors/attributes, EL/ED -----

    #[test]
    fn sgr_basic_16_foreground() {
        let rows = rows_of(80, &[b"\x1b[31mred"]);
        assert_eq!(rows[0].spans[0].style.fg, Color::Indexed(1));
        assert_eq!(rows[0].spans[0].text, "red");
    }

    #[test]
    fn sgr_bright_and_aliases() {
        let rows = rows_of(80, &[b"\x1b[92mA\x1b[101mB"]);
        // SGR changes are incremental: the foreground set by 92 persists
        // while 101 switches only the background.
        assert_eq!(rows[0].spans[0].style.fg, Color::Indexed(10));
        assert_eq!(rows[0].spans[0].style.bg, Color::Default);
        assert_eq!(rows[0].spans[1].style.fg, Color::Indexed(10));
        assert_eq!(rows[0].spans[1].style.bg, Color::Indexed(9));
    }

    #[test]
    fn sgr_256_palette() {
        let rows = rows_of(80, &[b"\x1b[38;5;196mfg\x1b[48;5;21mbg"]);
        // 48 replaces only the background; the foreground persists.
        assert_eq!(rows[0].spans[0].style.fg, Color::Indexed(196));
        assert_eq!(rows[0].spans[1].style.fg, Color::Indexed(196));
        assert_eq!(rows[0].spans[1].style.bg, Color::Indexed(21));
    }

    #[test]
    fn sgr_truecolor() {
        let rows = rows_of(80, &[b"\x1b[38;2;255;10;20;48;2;1;2;3mX"]);
        assert_eq!(
            rows[0].spans[0].style.fg,
            Color::Rgb {
                r: 255,
                g: 10,
                b: 20
            }
        );
        assert_eq!(rows[0].spans[0].style.bg, Color::Rgb { r: 1, g: 2, b: 3 });
    }

    #[test]
    fn sgr_attributes_and_full_reset() {
        let rows = rows_of(80, &[b"\x1b[1m\x1b[3m\x1b[4mb\x1b[0mplain"]);
        let first = &rows[0];
        assert!(first.spans[0].style.bold);
        assert!(first.spans[0].style.italic);
        assert!(first.spans[0].style.underline);
        assert!(first.spans[1].style.is_plain());
        assert_eq!(text(first), "bplain");
    }

    #[test]
    fn sgr_malformed_sequences_degrade_without_panic() {
        let rows = rows_of(
            80,
            &[b"\x1b[38;5mX\x1b[38;2;1;2mY\x1b[999mZ\x1b]0;title\x07W"],
        );
        let single = rows.iter().map(text).collect::<Vec<_>>().join("");
        assert_eq!(single, "XYZW");
        assert!(rows[0].spans[0].style.is_plain());
    }

    #[test]
    fn el_zero_erases_to_end_of_row() {
        // Cursor moves within the subset only (BS): from column 6 back to
        // 4; the erased range becomes painted blanks (default bg here).
        let rows = rows_of(10, &[b"abcdef\x08\x08\x1b[K"]);
        assert_eq!(text(&rows[0]), "abcd  ");
    }

    #[test]
    fn el_one_erases_through_cursor_inclusively() {
        let rows = rows_of(10, &[b"abcdef\x08\x08\x08\x1b[1K"]);
        assert_eq!(text(&rows[0]), "    ef");
    }

    #[test]
    fn ed_two_clears_current_row_only() {
        let rows = rows_of(10, &[b"abcdef\x1b[2J\nX"]);
        // ED 2 blanks the current logical row; the cursor does NOT move,
        // so the LF keeps column 6 and X lands there (staircase).
        assert_eq!(
            rows.iter().map(text).collect::<Vec<_>>(),
            ["      ", "      X"]
        );
    }

    #[test]
    fn ed_three_scrollback_clear_is_ignored() {
        let rows = rows_of(10, &[b"abc\x1b[3Jd"]);
        assert_eq!(text(&rows[0]), "abcd");
    }

    #[test]
    fn erase_paints_with_active_background() {
        // EL 0 from column 0: both cells become blanks carrying the bg
        // active at erase time (green here).
        let rows = rows_of(10, &[b"\x1b[42mAB\x08\x08\x1b[K"]);
        for span in &rows[0].spans {
            assert_eq!(span.style.bg, Color::Indexed(2));
        }
        assert_eq!(text(&rows[0]), "  ");
    }

    #[test]
    fn resize_updates_wrap_width_for_new_bytes() {
        let mut writer = BlockTextWriter::new(10);
        writer.write(b"verylongline\r\n");
        // 12 columns of text wrap at width 10 into two committed rows;
        // they keep their creation-time width (no reflow, BR-7).
        let rows = writer.take_committed_rows();
        assert_eq!(
            rows.iter().map(text).collect::<Vec<_>>(),
            ["verylongli", "ne"]
        );

        writer.resize_cols(40);
        assert_eq!(writer.cols(), 40);
        writer.write(b"short\n");
        // New bytes wrap at the NEW width — "short" fits in one row.
        let rows = writer.take_committed_rows();
        assert_eq!(rows.iter().map(text).collect::<Vec<_>>(), ["short"]);
    }
}
