//! Key passthrough and clipboard copy handling.

use gpui::*;

use super::TerminalWindow;
use crate::domain::block::{BlockId, CopyKind};

impl TerminalWindow {
    /// Keyboard passthrough: the shell keeps its own prompt and line editing,
    /// so keys go to the PTY as bytes. No local input buffer, no ghost prompt.
    pub(super) fn handle_key(&mut self, event: &KeyDownEvent) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;

        if modifiers.control {
            match keystroke.key.as_str() {
                "c" => self.write_keys(b"\x03"),
                "d" => self.write_keys(b"\x04"),
                "l" => self.write_keys(b"\x0c"),
                _ => {}
            }
            return;
        }

        match keystroke.key.as_str() {
            "enter" => self.write_keys(b"\r"),
            "backspace" => self.write_keys(b"\x7f"),
            "up" => self.write_keys(b"\x1b[A"),
            "down" => self.write_keys(b"\x1b[B"),
            "right" => self.write_keys(b"\x1b[C"),
            "left" => self.write_keys(b"\x1b[D"),
            _ => {
                // Modified keys outside the passthrough table are dropped
                // silently rather than encoded wrongly.
                if modifiers.alt || modifiers.platform || modifiers.function {
                    return;
                }
                if let Some(text) = keystroke.key_char.as_deref()
                    && !text.is_empty()
                    && text.chars().all(|c| !c.is_control())
                {
                    self.write_keys(text.as_bytes());
                }
            }
        }
    }

    /// Routes bytes through the application layer, never the PTY directly.
    fn write_keys(&mut self, bytes: &[u8]) {
        if let Err(error) = self.session_service.write_keystrokes(bytes) {
            tracing::warn!(error = %error, "keystroke write failed");
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
