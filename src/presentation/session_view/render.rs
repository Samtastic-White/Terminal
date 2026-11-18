//! Render implementation: layout, block cards, the editor dock, focus and
//! scroll wiring, and PTY size estimation.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::SystemTime;

use gpui::*;
use tracing::trace;

use super::{SelectionOrigin, TerminalWindow};
use crate::domain::block::{Block, BlockId, CopyKind};
use crate::domain::editor::MAX_VISIBLE_LINES;
use crate::domain::size::TerminalSize;
use crate::presentation::components::block_card::{
    ActionScope, BlockUiState, CardCallbacks, CopyHandler, SelectionHandler, render_block_card,
    render_header,
};
use crate::presentation::components::input_editor::{
    EDITOR_PADDING_X, EDITOR_PADDING_Y, render_input_editor,
};
use crate::presentation::components::row::render_rows;
use crate::presentation::components::sticky_header::{CardGeometry, header_height, resolve_pin};

/// Outer list padding plus card padding, border and scrollbar allowance, so
/// pre-wrapped rows fit inside cards without horizontal clipping.
const WINDOW_PADDING_X: f32 = 24.0;
const CARD_PADDING_X: f32 = 24.0;
const CARD_BORDER_X: f32 = 2.0;
const SCROLLBAR_ALLOWANCE: f32 = 12.0;
const OUTPUT_PADDING_X: f32 =
    WINDOW_PADDING_X + CARD_PADDING_X + CARD_BORDER_X + SCROLLBAR_ALLOWANCE;
const OUTPUT_PADDING_Y: f32 = 16.0;
/// The dock's top border, excluded from the canvas height.
const EDITOR_BORDER_Y: f32 = 1.0;
/// Rows of the block list the dock never covers. List rows and PTY rows share
/// the line height, so this is measured in editor lines.
const MIN_LIST_ROWS: usize = 6;

impl TerminalWindow {
    /// Recomputes editor geometry and cols/rows from font metrics and forwards
    /// a PTY resize only when the size actually changed. No debounce; stored
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
        let dock_chrome = 2.0 * EDITOR_PADDING_Y + EDITOR_BORDER_Y;

        let bounds = window.bounds();
        // The 28-line cap yields to short windows: the dock keeps room for a
        // few list rows and the editor scrolls earlier instead.
        let max_editor_rows =
            ((bounds.size.height.as_f32() - line_height * MIN_LIST_ROWS as f32 - dock_chrome)
                / line_height)
                .floor()
                .clamp(1.0, MAX_VISIBLE_LINES as f32) as usize;
        let editor_width = (bounds.size.width.as_f32() - 2.0 * EDITOR_PADDING_X).max(1.0);
        let editor_cols = (editor_width / char_width)
            .floor()
            .clamp(1.0, u16::MAX as f32) as usize;
        self.editor_char_width = char_width;
        self.session_service
            .set_editor_geometry(editor_cols, max_editor_rows);
        let editor_rows = self
            .session_service
            .editor_view()
            .total_lines
            .clamp(1, max_editor_rows);
        self.session_service
            .set_editor_geometry(editor_cols, editor_rows);
        let dock_height = line_height * editor_rows as f32 + dock_chrome;

        let available_width = (bounds.size.width.as_f32() - OUTPUT_PADDING_X).max(1.0);
        let available_height =
            (bounds.size.height.as_f32() - OUTPUT_PADDING_Y - dock_height).max(1.0);
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
        crate::probe::record_frame();

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
        let live_row = self.session_service.live_row();
        let live_ids: HashSet<BlockId> = blocks.iter().map(Block::id).collect();
        self.block_ui.retain(|id, _| live_ids.contains(id));

        let now = SystemTime::now();
        let view = cx.entity_id();

        let content = div()
            .id("terminal-output")
            .flex()
            .flex_col()
            .items_start()
            .flex_1()
            .min_h_0()
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
                let select_listener =
                    cx.listener(move |this: &mut TerminalWindow, _: &(), _window, cx| {
                        this.last_selection = Some(SelectionOrigin::Block(id));
                        cx.notify();
                    });
                let on_select: SelectionHandler =
                    Rc::new(move |window, cx| select_listener(&(), window, cx));
                let callbacks = CardCallbacks { on_copy, on_select };
                render_block_card(
                    block,
                    live_row.as_ref(),
                    &self.theme,
                    now,
                    ui,
                    view,
                    callbacks,
                )
            }));

        let line_height = px(self.theme.font_size * self.theme.line_height);
        // Bounds describe the previous frame, so the first frame after a
        // scroll still sees old geometry. `resolve_pin` then returns `None`
        // instead of pinning the wrong card (accepted one-frame lag, D7).
        let geometry: Vec<CardGeometry> = (0..blocks.len())
            .filter_map(|index| self.scroll.bounds_for_item(index))
            .map(|bounds| CardGeometry {
                top: bounds.top().as_f32(),
                bottom: bounds.bottom().as_f32(),
            })
            .collect();
        let pin = if geometry.len() == blocks.len() && !geometry.is_empty() {
            resolve_pin(
                &geometry,
                self.scroll.offset().y.as_f32(),
                self.scroll.bounds().top().as_f32(),
                header_height(self.theme.font_size, self.theme.line_height),
            )
        } else {
            None
        };
        let pinned_id = pin.and_then(|pin| blocks.get(pin.index)).map(Block::id);
        if pinned_id != self.pinned_block {
            match pinned_id {
                Some(id) => trace!(block_id = id.get(), "pinned header changed"),
                None => trace!("pinned header changed"),
            }
            self.pinned_block = pinned_id;
        }

        let mut scroll_region = div()
            .id("terminal-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .child(content);

        if let Some(pin) = pin
            && let Some(block) = blocks.get(pin.index)
        {
            let id = block.id();
            if let Some(snapshot) = self.block_ui.get(&id).and_then(|ui| ui.borrow().snapshot()) {
                let listener = cx.listener(
                    move |this: &mut TerminalWindow, kind: &CopyKind, _window, cx| {
                        this.copy_block(id, *kind, cx);
                    },
                );
                let on_copy: CopyHandler =
                    Rc::new(move |kind, window, cx| listener(&kind, window, cx));
                let group: SharedString = format!("block-{}", id.get()).into();
                let overlay = div()
                    .id("pinned-header")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .px_3()
                    .on_scroll(cx.listener(move |this, event: &ScrollEvent, _window, cx| {
                        let delta = event.delta.pixel_delta(line_height).y.as_f32();
                        let current = this.scroll.offset().y.as_f32();
                        let max = this.scroll.max_offset().y.as_f32();
                        this.scroll
                            .set_offset(point(px(0.0), px((current + delta).clamp(-max, 0.0))));
                        this.follow_tail = false;
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
                            .relative()
                            .top(px(pin.offset))
                            .rounded_md()
                            .border_1()
                            .border_color(self.theme.card_border)
                            .bg(self.theme.card_background)
                            .shadow_sm()
                            .group(group)
                            .child(render_header(
                                &snapshot,
                                id,
                                ActionScope::Pinned,
                                &self.theme,
                                &on_copy,
                            )),
                    );
                scroll_region = scroll_region.child(overlay);
            }
        }

        let editor_rows = self.session_service.editor_visual_lines();
        let editor_view = Rc::new(self.session_service.editor_view());
        let dock_height =
            line_height * editor_rows as f32 + px(2.0 * EDITOR_PADDING_Y + EDITOR_BORDER_Y);
        let feedback = self.session_service.feedback_rows();
        let mut dock = div()
            .id("editor-dock")
            .relative()
            .flex_none()
            .w_full()
            .px(px(EDITOR_PADDING_X))
            .py(px(EDITOR_PADDING_Y))
            .border_t_1()
            .border_color(self.theme.card_border)
            .bg(self.theme.card_background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus_handle, cx);
                    this.editor_mouse_down(event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.editor_mouse_move(event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| this.editor_mouse_up(cx)),
            )
            .on_scroll(cx.listener(move |this, event: &ScrollEvent, _window, cx| {
                let delta = event.delta.pixel_delta(line_height).y.as_f32();
                if this.editor_scroll(delta, line_height, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(render_input_editor(
                self.editor_layout.clone(),
                editor_view,
                &self.theme,
                editor_rows,
            ));

        if !feedback.is_empty() {
            dock = dock.child(
                div()
                    .id("editor-feedback")
                    .absolute()
                    .left(px(EDITOR_PADDING_X))
                    .right(px(EDITOR_PADDING_X))
                    .bottom(dock_height)
                    .max_h(line_height * 3.0)
                    .overflow_hidden()
                    .rounded_md()
                    .border_1()
                    .border_color(self.theme.card_border)
                    .bg(self.theme.card_background)
                    .shadow_sm()
                    .px(px(12.0))
                    .py(px(4.0))
                    .child(render_rows(feedback.iter(), None, &self.theme)),
            );
        }

        div()
            .id("terminal-window")
            .flex()
            .flex_col()
            .size_full()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event, _window, cx| {
                this.handle_key(event, cx);
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
            .child(scroll_region)
            .child(dock)
    }
}
