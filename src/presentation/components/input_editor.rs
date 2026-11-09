//! Input Editor — native GPUI multi-line editor (Phase 1: basic passthrough).
//!
//! Handles printable keys, Enter, Backspace, Ctrl+C, Ctrl+D, Ctrl+L, arrows.
//! Routes keystrokes to Application layer via callback.

use gpui::*;
use crate::presentation::theme::TerminalTheme;
use crate::presentation::theme::colors;
use crate::common::errors::Result;
use std::sync::Arc;

/// Simple input buffer for Phase 1 passthrough
struct InputBuffer {
    content: String,
    cursor_pos: usize,
}

impl Default for InputBuffer {
    fn default() -> Self {
        Self { content: String::new(), cursor_pos: 0 }
    }
}

impl InputBuffer {
    fn insert_char(&mut self, c: char) {
        self.content.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            // Find previous char boundary
            let prev = self.content[..self.cursor_pos].chars().rev().next();
            if let Some(c) = prev {
                let char_len = c.len_utf8();
                self.cursor_pos -= char_len;
                self.content.drain(self.cursor_pos..self.cursor_pos + char_len);
            }
        }
    }

    fn take_line(&mut self) -> Option<String> {
        if self.content.contains('\n') {
            // Multi-line: take up to first newline
            let idx = self.content.find('\n').unwrap();
            let line = self.content[..idx].to_string();
            self.content.drain(..=idx);
            self.cursor_pos = 0;
            Some(line)
        } else if !self.content.is_empty() {
            // Single line: take all on Enter
            let line = std::mem::take(&mut self.content);
            self.cursor_pos = 0;
            Some(line)
        } else {
            None
        }
    }
}

/// InputEditor component — handles keyboard input and renders current buffer
pub struct InputEditor {
    focus_handle: FocusHandle,
    buffer: InputBuffer,
    theme: TerminalTheme,
    on_submit: Arc<dyn Fn(String) -> Result<()> + Send + Sync>,
    on_ctrl_c: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    on_ctrl_d: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    on_ctrl_l: Arc<dyn Fn() -> Result<()> + Send + Sync>,
}

impl InputEditor {
    pub fn new(
        theme: TerminalTheme,
        on_submit: impl Fn(String) -> Result<()> + Send + Sync + 'static,
        on_ctrl_c: impl Fn() -> Result<()> + Send + Sync + 'static,
        on_ctrl_d: impl Fn() -> Result<()> + Send + Sync + 'static,
        on_ctrl_l: impl Fn() -> Result<()> + Send + Sync + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            buffer: InputBuffer::default(),
            theme,
            on_submit: Arc::new(on_submit),
            on_ctrl_c: Arc::new(on_ctrl_c),
            on_ctrl_d: Arc::new(on_ctrl_d),
            on_ctrl_l: Arc::new(on_ctrl_l),
        }
    }

    /// Get the current input text (for rendering)
    fn current_input(&self) -> &str {
        &self.buffer.content
    }
}

impl Focusable for InputEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InputEditor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let input_text = self.current_input().to_string();
        let cursor_pos = self.buffer.cursor_pos;

        // Build cursor position visual
        let (before_cursor, after_cursor) = input_text.split_at(cursor_pos.min(input_text.len()));

        let mut root = div()
            .id("input-editor")
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .bg(self.theme.background)
            .text_color(self.theme.foreground)
            .border_t_1()
            .border_color(colors::SURFACE0)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_baseline()
                            .child("❯ ") // Prompt indicator
                            .text_color(colors::GREEN)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_baseline()
                                    .child(before_cursor)
                                    .child(
                                        // Cursor block
                                        div()
                                            .id("cursor")
                                            .w(px(2.0))
                                            .h(px(self.theme.font_size * self.theme.line_height))
                                            .bg(self.theme.cursor)
                                    )
                                    .child(after_cursor)
                            )
                    )
            );

        if input_text.is_empty() {
            root = root.child(div().flex_1().child(" "));
        }

        root
    }
}

impl InputEditor {
    /// Handle key down events
    pub fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let key = keystroke.key;

        // Ctrl combinations
        if modifiers.control {
            match key {
                Key::Character(c) if c == "c" => {
                    let _ = (self.on_ctrl_c)();
                    return;
                }
                Key::Character(c) if c == "d" => {
                    let _ = (self.on_ctrl_d)();
                    return;
                }
                Key::Character(c) if c == "l" => {
                    let _ = (self.on_ctrl_l)();
                    return;
                }
                _ => {}
            }
        }

        // Regular keys
        match key {
            Key::Enter => {
                if let Some(line) = self.buffer.take_line() {
                    let line_with_newline = format!("{}\r", line);
                    let _ = (self.on_submit)(line_with_newline);
                } else {
                    // Empty Enter - send just CR
                    let _ = (self.on_submit)("\r".to_string());
                }
            }
            Key::Backspace => {
                self.buffer.backspace();
            }
            Key::Character(c) => {
                // Printable character
                if c.len() == 1 {
                    self.buffer.insert_char(c.chars().next().unwrap());
                }
            }
            Key::ArrowLeft => {
                if self.buffer.cursor_pos > 0 {
                    // Move back one char boundary
                    let before = &self.buffer.content[..self.buffer.cursor_pos];
                    if let Some(last_char) = before.chars().rev().next() {
                        self.buffer.cursor_pos -= last_char.len_utf8();
                    }
                }
            }
            Key::ArrowRight => {
                let after = &self.buffer.content[self.buffer.cursor_pos..];
                if let Some(next_char) = after.chars().next() {
                    self.buffer.cursor_pos += next_char.len_utf8();
                }
            }
            Key::ArrowUp | Key::ArrowDown => {
                // Arrow keys: send as CSI sequences (plain CSI, not SS3)
                // Phase 1: simple CSI encoding
                let seq = match key {
                    Key::ArrowUp => "\x1b[A",
                    Key::ArrowDown => "\x1b[B",
                    Key::ArrowRight => "\x1b[C",
                    Key::ArrowLeft => "\x1b[D",
                    _ => return,
                };
                let _ = (self.on_submit)(seq.to_string());
            }
            _ => {}
        }
    }
}