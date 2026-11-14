//! Render implementation: layout, block cards, prompt rows, focus and scroll
//! wiring, and PTY size estimation.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::SystemTime;

use gpui::*;

use super::TerminalWindow;
use crate::domain::block::{Block, BlockId, CopyKind};
use crate::domain::semantic::ShellPhase;
use crate::domain::size::TerminalSize;
use crate::presentation::components::block_card::{BlockUiState, CopyHandler, render_block_card};
use crate::presentation::components::row::render_rows;

/// Outer list padding plus card padding, border and scrollbar allowance, so
/// pre-wrapped rows fit inside cards without horizontal clipping.
const WINDOW_PADDING_X: f32 = 24.0;
const CARD_PADDING_X: f32 = 24.0;
const CARD_BORDER_X: f32 = 2.0;
const SCROLLBAR_ALLOWANCE: f32 = 12.0;
const OUTPUT_PADDING_X: f32 =
    WINDOW_PADDING_X + CARD_PADDING_X + CARD_BORDER_X + SCROLLBAR_ALLOWANCE;
const OUTPUT_PADDING_Y: f32 = 16.0;

impl TerminalWindow {
    /// Recomputes cols/rows from font metrics and forwards only when the size
    /// actually changed. No debounce, but no per-frame ioctl either; stored
    /// rows keep their creation-time wrapping.
    fn update_pty_size(&mut self, window: &Window) {
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&gpui::font(self.theme.font_family));
        let char_width = text_system
            .ch_advance(font_id, px(self.theme.font_size))
            .map(|width| width.as_f32())
            .unwrap_or(self.theme.font_size * 0.6)
            .max(1.0);
        let line_height = self.theme.font_size * self.theme.line_height;

        let bounds = window.bounds();
        let available_width = (bounds.size.width.as_f32() - OUTPUT_PADDING_X).max(1.0);
        let available_height = (bounds.size.height.as_f32() - OUTPUT_PADDING_Y).max(1.0);
        let cols = (available_width / char_width)
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16;
        let rows = (available_height / line_height)
            .floor()
            .clamp(1.0, u16::MAX as f32) as u16;
        let size = TerminalSize::new(cols, rows);

        if self.last_terminal_size != Some(size) {
            self.last_terminal_size = Some(size);
            if let Err(error) = self.session_service.resize(size) {
                tracing::warn!(error = %error, "resize forward failed");
            }
        }
    }

    fn is_at_bottom(&self) -> bool {
        self.scroll.offset().y + self.scroll.max_offset().y <= px(4.0)
    }
}

impl Focusable for TerminalWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.did_focus {
            window.focus(&self.focus_handle, cx);
            self.did_focus = true;
        }

        self.update_pty_size(window);

        // Follow the tail while docked; a manual scroll suspends it and
        // returning to the end resumes it.
        if self.follow_tail {
            self.scroll.scroll_to_bottom();
        } else if self.is_at_bottom() {
            self.follow_tail = true;
            self.scroll.scroll_to_bottom();
        }

        let blocks = self.session_service.blocks();
        let live_ids: HashSet<BlockId> = blocks.iter().map(Block::id).collect();
        self.block_ui.retain(|id, _| live_ids.contains(id));

        let prompt_rows = self.session_service.prompt_rows();
        let live = self.session_service.live_row();
        let prompt_visible = !matches!(self.session_service.phase(), ShellPhase::Closed);
        let now = SystemTime::now();
        let view = cx.entity_id();

        let content = div()
            .id("terminal-output")
            .flex()
            .flex_col()
            .items_start()
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px_3()
            .py_2()
            .children(blocks.iter().map(|block| {
                let id = block.id();
                let ui = self
                    .block_ui
                    .entry(id)
                    .or_insert_with(|| Rc::new(RefCell::new(BlockUiState::new())))
                    .clone();
                let listener = cx.listener(
                    move |this: &mut TerminalWindow, kind: &CopyKind, _window, cx| {
                        this.copy_block(id, *kind, cx);
                    },
                );
                let on_copy: CopyHandler =
                    Rc::new(move |kind, window, cx| listener(&kind, window, cx));
                render_block_card(block, &self.theme, now, ui, view, on_copy)
            }));
        let content = if prompt_visible {
            content.child(render_rows(prompt_rows.iter(), live.as_ref(), &self.theme))
        } else {
            content
        };

        div()
            .id("terminal-window")
            .flex()
            .flex_col()
            .size_full()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, _window, _cx| {
                this.handle_key(event);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .on_scroll(cx.listener(|this, _event, _window, cx| {
                this.follow_tail = false;
                cx.notify();
            }))
            .child(content)
    }
}
