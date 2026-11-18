//! Key dispatch, editor pointer handling, and clipboard operations. Every
//! byte reaches the PTY through the application service.

use gpui::*;
use tracing::{debug, trace, warn};

use super::TerminalWindow;
use super::keys::key_chord;
use crate::application::session::{EditCommand, EditorState};
use crate::domain::block::{BlockId, CopyKind};
use crate::domain::keymap::KeyToken;

impl TerminalWindow {
    /// One dispatcher keyed by the derived editor state.
    pub(super) fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        match self.session_service.editor_state() {
            EditorState::DisabledCold | EditorState::DisabledClosed => {}
            EditorState::Idle => self.handle_idle_key(keystroke, cx),
            EditorState::AwaitingStart | EditorState::Running => {
                self.handle_locked_key(keystroke, cx);
            }
        }
    }

    fn handle_idle_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let modifiers = keystroke.modifiers;
        let key = keystroke.key.as_str();

        if modifiers.control {
            if modifiers.shift {
                match key {
                    "c" => self.copy_selection(cx),
                    "v" => self.paste_clipboard(cx),
                    _ => {}
                }
            } else {
                match key {
                    "a" => self.edit(EditCommand::SelectAll, cx),
                    "c" => {
                        self.session_service.interrupt();
                        cx.notify();
                    }
                    "d" => {
                        if self.session_service.editor().text().is_empty() {
                            self.write_keys(&[0x04]);
                        } else {
                            self.edit(EditCommand::Delete, cx);
                        }
                    }
                    "v" => self.paste_clipboard(cx),
                    _ => {}
                }
            }
            return;
        }

        if modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }

        let extend = modifiers.shift;
        match key {
            "enter" => {
                if modifiers.shift {
                    self.edit(EditCommand::Insert("\n".into()), cx);
                } else {
                    self.session_service.submit();
                    cx.notify();
                }
            }
            "backspace" => self.edit(EditCommand::Backspace, cx),
            "delete" => self.edit(EditCommand::Delete, cx),
            "left" => self.edit(EditCommand::MoveLeft { extend }, cx),
            "right" => self.edit(EditCommand::MoveRight { extend }, cx),
            "up" => self.edit(EditCommand::MoveUp { extend }, cx),
            "down" => self.edit(EditCommand::MoveDown { extend }, cx),
            "home" => self.edit(EditCommand::Home { extend }, cx),
            "end" => self.edit(EditCommand::End { extend }, cx),
            "pageup" => self.edit(EditCommand::PageUp, cx),
            "pagedown" => self.edit(EditCommand::PageDown, cx),
            "tab" if !modifiers.shift => self.edit(EditCommand::Insert("\t".into()), cx),
            "tab" | "backtab" | "escape" => {}
            _ => {
                if let Some(text) = keystroke.key_char.as_deref()
                    && !text.is_empty()
                {
                    self.edit(EditCommand::Insert(text.to_owned()), cx);
                }
            }
        }
    }

    fn handle_locked_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let modifiers = keystroke.modifiers;
        let key = keystroke.key.as_str();

        if modifiers.control {
            if modifiers.shift {
                match key {
                    "c" => {
                        self.copy_selection(cx);
                        return;
                    }
                    "v" => {
                        self.paste_clipboard(cx);
                        return;
                    }
                    _ => {}
                }
            }
            if key == "c" {
                self.session_service.interrupt();
                cx.notify();
                return;
            }
        }

        let Some(chord) = key_chord(keystroke) else {
            trace!(key = %key, "key unsupported");
            return;
        };
        match self.session_service.encode_key(&chord) {
            Some(bytes) => {
                // Named keys are safe to trace verbatim: they carry no typed
                // payload, and the bytes reveal which dialect was chosen.
                let encoded =
                    matches!(&chord.token, KeyToken::Named(_)).then(|| format!("{bytes:02x?}"));
                trace!(
                    key = %key,
                    bytes = bytes.len(),
                    encoded = encoded.as_deref(),
                    "key encoded"
                );
                self.write_keys(&bytes);
            }
            None => trace!(key = %key, "key unsupported"),
        }
    }

    fn edit(&mut self, command: EditCommand, cx: &mut Context<Self>) {
        self.session_service.edit(command);
        if self.session_service.editor().has_selection() {
            self.last_selection = Some(super::SelectionOrigin::Editor);
        }
        cx.notify();
    }

    /// Applies a click as a caret move and starts a drag from that caret.
    pub(super) fn editor_mouse_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let view = self.session_service.editor_view();
        let Some((line, cell)) = self.editor_layout.borrow().cell_at(position) else {
            return;
        };
        self.session_service.edit(EditCommand::MoveToCell {
            line: view.first_line + line,
            cell,
        });
        self.editor_layout.borrow_mut().begin_drag();
        cx.notify();
    }

    pub(super) fn editor_mouse_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.editor_layout.borrow().is_dragging() {
            return;
        }
        let view = self.session_service.editor_view();
        let Some((line, cell)) = self.editor_layout.borrow().cell_at(position) else {
            return;
        };
        self.session_service.edit(EditCommand::DragToCell {
            line: view.first_line + line,
            cell,
        });
        if self.session_service.editor().has_selection() {
            self.last_selection = Some(super::SelectionOrigin::Editor);
        }
        cx.notify();
    }

    pub(super) fn editor_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.editor_layout.borrow_mut().end_drag();
        self.editor_scroll_accum = 0.0;
        cx.notify();
    }

    /// Wheel over the editor scrolls its window when it overflows; otherwise
    /// the event bubbles to the block list.
    pub(super) fn editor_scroll(
        &mut self,
        delta_y: f32,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let view = self.session_service.editor_view();
        let visible = self.session_service.editor_visual_lines();
        if view.total_lines <= visible {
            return false;
        }
        self.editor_scroll_accum += delta_y;
        let line_px = line_height.as_f32().max(1.0);
        let steps = (self.editor_scroll_accum / line_px).trunc();
        if steps == 0.0 {
            return false;
        }
        self.editor_scroll_accum -= steps * line_px;
        if self.session_service.scroll_editor(steps as isize) {
            cx.notify();
            true
        } else {
            false
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let text = match self.last_selection {
            Some(super::SelectionOrigin::Editor) => self
                .session_service
                .editor()
                .selected_text()
                .map(str::to_owned),
            Some(super::SelectionOrigin::Block(id)) => self
                .block_ui
                .get(&id)
                .and_then(|ui| ui.borrow().selected_text()),
            None => None,
        };

        match text.filter(|text| !text.is_empty()) {
            Some(text) => {
                debug!(bytes = text.len(), "selection copied");
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            None => debug!(
                origin = ?self.last_selection,
                "editor copy skipped: empty"
            ),
        }
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return;
        };

        match self.session_service.editor_state() {
            EditorState::Idle => self.edit(EditCommand::Paste(text), cx),
            EditorState::AwaitingStart | EditorState::Running => {
                if let Err(error) = self.session_service.paste_to_pty(&text) {
                    warn!(error = %error, "editor paste failed");
                }
            }
            EditorState::DisabledCold | EditorState::DisabledClosed => {}
        }
    }

    /// Routes bytes through the application layer, never the PTY directly.
    pub(super) fn write_keys(&mut self, bytes: &[u8]) {
        if let Err(error) = self.session_service.write_keystrokes(bytes) {
            warn!(error = %error, "keystroke write failed");
        }
    }

    /// Extracts one clipboard surface from the owning block and writes it.
    /// Clipboard content itself is never logged.
    pub(super) fn copy_block(&mut self, id: BlockId, kind: CopyKind, cx: &mut App) {
        match self.session_service.copy_block(id, kind) {
            Some(text) => {
                tracing::debug!(
                    block_id = id.get(),
                    kind = ?kind,
                    bytes = text.len(),
                    "block copied"
                );
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            None => tracing::debug!(
                block_id = id.get(),
                kind = ?kind,
                "block copy skipped: evicted"
            ),
        }
    }
}
