//! Per-block interaction state: body cache, selection, scroll window, and
//! pending text layout. Owned by the window, wrapped in a `RefCell` by the
//! caller because GPUI listeners need shared mutable access.

use std::ops::Range;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::*;

use super::body::{BodyText, build_body_text};
use super::snapshot::{CardSnapshot, card_snapshot_with_relative};
use super::view::body_text_runs;
use crate::application::block::format_relative;
use crate::domain::block::{Block, BlockStatus};
use crate::presentation::theme::TerminalTheme;

/// Maximum visual lines a card body shows before scrolling locally.
const BODY_MAX_LINES: usize = 28;

/// Absolute byte selection in the flat body text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    anchor: usize,
    focus: usize,
}

impl Selection {
    fn range(self) -> Range<usize> {
        if self.anchor <= self.focus {
            self.anchor..self.focus
        } else {
            self.focus..self.anchor
        }
    }

    fn is_empty(self) -> bool {
        self.anchor == self.focus
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BodyKey {
    revision: u64,
    start_row: usize,
    end_row: usize,
    selection: Option<(usize, usize)>,
}

struct CachedBody {
    revision: u64,
    total: u64,
    body: BodyText,
}

/// Header snapshot cached until its inputs change: output revision, status,
/// command, or the wall-clock second the relative age is expressed in.
struct CachedHeader {
    revision: u64,
    status: BlockStatus,
    second: u64,
    command: String,
    snapshot: Rc<CardSnapshot>,
}

/// Slice waiting to be shaped on the next prepaint, so shaping happens with
/// the window text system without borrowing the window across render.
pub(super) struct PendingBody {
    pub(super) text: String,
    pub(super) runs: Vec<TextRun>,
    pub(super) line_height: Pixels,
    pub(super) slice_start_byte: usize,
    pub(super) slice_end_byte: usize,
}

pub(super) struct BodyLayout {
    pub(super) lines: Vec<WrappedLine>,
    pub(super) bounds: Option<Bounds<Pixels>>,
    pub(super) line_height: Pixels,
    pub(super) slice_start_byte: usize,
    pub(super) slice_end_byte: usize,
}

/// Interaction state owned by the window for one block. Wrapped in a
/// `RefCell` because GPUI listeners need shared mutable access.
pub struct BlockUiState {
    body_scroll: usize,
    scroll_accum: f32,
    follow_tail: bool,
    selection: Option<Selection>,
    dragging: bool,
    body: Option<CachedBody>,
    pub(super) pending: Option<PendingBody>,
    pub(super) layout: Option<BodyLayout>,
    layout_key: Option<BodyKey>,
    header: Option<CachedHeader>,
}

impl Default for BlockUiState {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockUiState {
    pub fn new() -> Self {
        Self {
            body_scroll: 0,
            scroll_accum: 0.0,
            follow_tail: true,
            selection: None,
            dragging: false,
            body: None,
            pending: None,
            layout: None,
            layout_key: None,
            header: None,
        }
    }

    /// Syncs the cached flat body with the block. Appends only the rows added
    /// since the last revision and drops evicted rows from the front, falling
    /// back to a full rebuild when the counters disagree. Appending output
    /// invalidates byte offsets, so any selection is dropped.
    pub(super) fn sync_body(&mut self, block: &Block) {
        let revision = block.output_revision();
        let total = block.output_total();
        if self
            .body
            .as_ref()
            .is_some_and(|cached| cached.revision == revision && cached.total == total)
        {
            return;
        }

        let rows = block.output();
        let row_count = rows.len();
        let previous = self.body.take();
        let body = match previous {
            Some(cached) if total > cached.total && revision > cached.revision => {
                let appended = (total - cached.total) as usize;
                let cached_rows = cached.body.row_count();
                if appended <= row_count && cached_rows + appended >= row_count {
                    let evicted = cached_rows + appended - row_count;
                    let mut body = cached.body;
                    body.drop_front(evicted);
                    for row in rows.iter().skip(row_count - appended) {
                        body.push_row(row);
                    }
                    body
                } else {
                    build_body_text(rows.iter())
                }
            }
            _ => build_body_text(rows.iter()),
        };
        self.body = Some(CachedBody {
            revision,
            total,
            body,
        });
        self.selection = None;
        self.dragging = false;
        self.pending = None;
        self.layout = None;
        self.layout_key = None;
        self.scroll_accum = 0.0;
    }

    /// Header snapshot with the relative age cached within one wall-clock
    /// second, so idle frames do not reallocate command/status/meta strings.
    pub(super) fn header_snapshot(&mut self, block: &Block, now: SystemTime) -> Rc<CardSnapshot> {
        let second = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let command = block.command().unwrap_or("…");
        if let Some(cached) = &self.header
            && cached.second == second
            && cached.revision == block.output_revision()
            && cached.status == block.status()
            && cached.command == command
        {
            return cached.snapshot.clone();
        }

        let relative = format_relative(block.started_wall(), now);
        let snapshot = Rc::new(card_snapshot_with_relative(block, relative));
        self.header = Some(CachedHeader {
            revision: block.output_revision(),
            status: block.status(),
            second,
            command: command.to_owned(),
            snapshot: snapshot.clone(),
        });
        snapshot
    }

    /// Positions the visible window, queues a slice when its key changed, and
    /// returns the body height in pixels. Rows are pre-wrapped to the PTY
    /// width, so one row is always one visual line.
    pub(super) fn prepare_body(&mut self, theme: &TerminalTheme) -> Option<Pixels> {
        let revision = self.body.as_ref()?.revision;
        let rows = self.body.as_ref()?.body.row_count();
        if rows == 0 {
            return None;
        }

        let visible = rows.min(BODY_MAX_LINES);
        let max_scroll = rows - visible;
        if self.follow_tail {
            self.body_scroll = max_scroll;
        } else {
            self.body_scroll = self.body_scroll.min(max_scroll);
        }

        let end_row = self.body_scroll + visible;
        let selection = self
            .selection
            .filter(|selection| !selection.is_empty())
            .map(Selection::range);
        let key = BodyKey {
            revision,
            start_row: self.body_scroll,
            end_row,
            selection: selection.as_ref().map(|range| (range.start, range.end)),
        };
        let line_height = px(theme.font_size * theme.line_height);

        if self.layout_key != Some(key) || self.layout.is_none() {
            let slice = self
                .body
                .as_ref()?
                .body
                .slice(self.body_scroll, end_row, selection);
            let runs = body_text_runs(&slice, theme);
            let slice_start_byte = slice.start_byte();
            let slice_end_byte = slice_start_byte + slice.text().len();
            self.pending = Some(PendingBody {
                text: slice.text().to_owned(),
                runs,
                line_height,
                slice_start_byte,
                slice_end_byte,
            });
            self.layout = None;
            self.layout_key = Some(key);
        }

        Some(px(line_height.as_f32() * visible as f32))
    }

    /// Maps a window position over the body to an absolute byte offset in the
    /// flat body text.
    fn index_at(&self, position: Point<Pixels>) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let bounds = layout.bounds?;
        if position.y < bounds.top() {
            return Some(layout.slice_start_byte);
        }

        let mut top = bounds.top();
        let mut line_start = 0usize;
        for line in &layout.lines {
            let bottom = top + line.size(layout.line_height).height;
            if position.y < bottom {
                let local = point(
                    (position.x - bounds.left()).max(px(0.0)),
                    (position.y - top).max(px(0.0)),
                );
                let within = match line.index_for_position(local, layout.line_height) {
                    Ok(index) | Err(index) => index,
                };
                return Some(
                    (layout.slice_start_byte + line_start + within).min(layout.slice_end_byte),
                );
            }
            top = bottom;
            line_start += line.len() + 1;
        }
        Some(layout.slice_end_byte)
    }

    /// Starts a new selection at `position`. The anchor and focus coincide
    /// until the pointer moves, so a click leaves no painted range.
    pub(super) fn begin_selection(&mut self, position: Point<Pixels>) -> bool {
        let Some(index) = self.index_at(position) else {
            return false;
        };
        self.selection = Some(Selection {
            anchor: index,
            focus: index,
        });
        self.dragging = true;
        true
    }

    pub(super) fn extend_selection(&mut self, position: Point<Pixels>) -> bool {
        if !self.dragging {
            return false;
        }
        let Some(index) = self.index_at(position) else {
            return false;
        };
        let Some(selection) = self.selection.as_mut() else {
            return false;
        };
        if selection.focus == index {
            return false;
        }
        selection.focus = index;
        true
    }

    pub(super) fn end_selection(&mut self) -> bool {
        let was_dragging = self.dragging;
        self.dragging = false;
        was_dragging
    }

    /// Scrolls the local body window by a pixel delta. Returns true when the
    /// visible rows moved; refusals let the outer list scroll instead.
    pub(super) fn scroll_by(&mut self, delta_y: f32, line_height: Pixels) -> bool {
        let rows = self
            .body
            .as_ref()
            .map(|cached| cached.body.row_count())
            .unwrap_or(0);
        let max_scroll = rows.saturating_sub(BODY_MAX_LINES);
        if max_scroll == 0 {
            self.follow_tail = true;
            return false;
        }

        let line_px = line_height.as_f32().max(1.0);
        self.scroll_accum += delta_y;
        let steps = (self.scroll_accum / line_px).trunc();
        if steps == 0.0 {
            return false;
        }
        self.scroll_accum -= steps * line_px;

        let next =
            (self.body_scroll as isize + steps as isize).clamp(0, max_scroll as isize) as usize;
        self.follow_tail = next == max_scroll;
        if next == self.body_scroll {
            self.scroll_accum = 0.0;
            return false;
        }
        self.body_scroll = next;
        true
    }
}
