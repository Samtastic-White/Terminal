//! OSC dispatch: shell-integration markers (133) and VS Code convention
//! payloads (633), plus active-row retirement around semantic markers.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use tracing::trace;

use super::Core;
use crate::domain::output::TextStyle;
use crate::domain::semantic::SemanticEvent;
use crate::domain::terminal_surface::SurfaceItem;

impl Core {
    /// A row holding only blanks with the cursor back at column zero is the
    /// erased ZLE line-clear marker (`%` plus padding, then CR), not output.
    /// ZLE emits it between `C` and `D`; a faithful grid shows nothing
    /// because the next prompt repaints the line.
    fn is_cleared_blank(&self) -> bool {
        self.col == 0
            && self
                .active
                .iter()
                .flatten()
                .all(|cell| cell.ch == ' ' && !cell.cont && cell.style == TextStyle::default())
    }

    /// Commits a written active row without fabricating blank rows, so rows
    /// that precede a semantic marker keep their stream position. The marker
    /// starts a new logical line, so the cursor returns to column zero.
    pub(super) fn flush_active(&mut self) {
        if self.is_cleared_blank() {
            self.active.clear();
        } else if self.active.iter().any(|cell| cell.is_some()) {
            self.commit();
        }
        self.col = 0;
        self.pending_wrap = false;
    }

    /// Emits a semantic event after committing any row already displayed.
    pub(super) fn emit(&mut self, event: SemanticEvent) {
        self.flush_active();
        self.items.push_back(SurfaceItem::Semantic(event));
    }

    /// OSC dispatch: only the documented semantic payloads are recognized;
    /// every other OSC is consumed silently.
    pub(super) fn handle_osc(&mut self, params: &[&[u8]]) {
        let Some((kind, rest)) = params.split_first() else {
            trace!(family = "osc", "parser input ignored");
            return;
        };
        match *kind {
            b"133" => self.handle_133(rest),
            b"633" => self.handle_633(rest),
            _ => trace!(family = "osc", "parser input ignored"),
        }
    }

    /// `133;A|B|C` and `133;D;<code>`: exact arity, numeric status only.
    fn handle_133(&mut self, params: &[&[u8]]) {
        let Some(action) = params.first().copied() else {
            trace!(family = "osc", "parser input ignored");
            return;
        };
        if action == b"A" && params.len() == 1 {
            self.emit(SemanticEvent::PromptStart);
        } else if action == b"B" && params.len() == 1 {
            self.emit(SemanticEvent::PromptEnd);
        } else if action == b"C" && params.len() == 1 {
            self.emit(SemanticEvent::CommandStart);
        } else if action == b"D" && params.len() == 2 {
            match parse_exit_code(params[1]) {
                Some(exit_code) => self.emit(SemanticEvent::CommandFinished { exit_code }),
                None => trace!(family = "osc", "parser input ignored"),
            }
        } else {
            trace!(family = "osc", "parser input ignored");
        }
    }

    /// `633;P;Cwd=<path>` and `633;E;<command>`: take-rest payloads.
    fn handle_633(&mut self, params: &[&[u8]]) {
        let Some(kind) = params.first().copied() else {
            trace!(family = "osc", "parser input ignored");
            return;
        };
        let payload = join_params(&params[1..]);
        match kind {
            b"E" => {
                let text = sanitize_command(&payload);
                if text.is_empty() {
                    trace!(family = "osc", "parser input ignored");
                } else {
                    self.emit(SemanticEvent::CommandLine { text });
                }
            }
            b"P" => {
                let Some(path) = payload.strip_prefix(b"Cwd=") else {
                    trace!(family = "osc", "parser input ignored");
                    return;
                };
                if path.is_empty() {
                    trace!(family = "osc", "parser input ignored");
                    return;
                }
                self.emit(SemanticEvent::DirectoryChanged {
                    path: PathBuf::from(OsString::from_vec(path.to_vec())),
                });
            }
            _ => trace!(family = "osc", "parser input ignored"),
        }
    }
}

/// Rebuilds a take-rest payload from `vte`'s semicolon-split parameters.
fn join_params(params: &[&[u8]]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            payload.push(b';');
        }
        payload.extend_from_slice(param);
    }
    payload
}

/// Removes bytes that cannot appear in a header line; `vte` already drops C0
/// controls inside OSC, so this mainly guards DEL and invalid UTF-8.
fn sanitize_command(bytes: &[u8]) -> String {
    let mut cleaned = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if !byte.is_ascii_control() {
            cleaned.push(byte);
        }
    }
    String::from_utf8_lossy(&cleaned).into_owned()
}

fn parse_exit_code(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}
