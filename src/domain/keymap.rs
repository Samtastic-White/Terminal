//! Key vocabulary and byte encoding for the foreground terminal application.
//!
//! The editor consumes keys locally while it is idle; once a command runs the
//! same keys are encoded in the dialect the foreground program negotiated
//! through its terminal modes. Encoding is pure and has no OS or GUI
//! dependency.

// The session orchestration routes keys through this encoder; until that
// wiring exists the unit tests are the only users in a non-test build.
#![cfg_attr(not(test), allow(dead_code))]

const ESC: u8 = 0x1b;
const DEL: u8 = 0x7f;

/// Terminal modes that change how keys are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalModes {
    app_cursor: bool,
}

impl TerminalModes {
    /// DECCKM state, set by `CSI ? 1 h` and cleared by `CSI ? 1 l`.
    pub fn app_cursor(self) -> bool {
        self.app_cursor
    }

    pub fn set_app_cursor(&mut self, enabled: bool) {
        self.app_cursor = enabled;
    }

    /// Clears every tracked mode, as a terminal reset (`ESC c`) does.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Keys whose legacy encoding is a fixed escape sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKey {
    Enter,
    Backspace,
    Tab,
    BackTab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
}

impl NamedKey {
    /// Maps a platform key name to the vocabulary; function keys and unknown
    /// names have no representation and return `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "enter" => Self::Enter,
            "backspace" => Self::Backspace,
            "tab" => Self::Tab,
            "backtab" => Self::BackTab,
            "escape" => Self::Escape,
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" => Self::PageUp,
            "pagedown" => Self::PageDown,
            "insert" => Self::Insert,
            "delete" => Self::Delete,
            _ => return None,
        })
    }
}

/// What a keystroke carries: one char, composed text, or a named key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyToken {
    Char(char),
    Text(String),
    Named(NamedKey),
}

/// A platform-independent keystroke ready for encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub token: KeyToken,
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

/// Encodes a chord for the foreground program. `None` means the chord has no
/// legacy representation and the caller traces and drops it.
pub fn encode_key(chord: &KeyChord, modes: TerminalModes) -> Option<Vec<u8>> {
    match &chord.token {
        KeyToken::Char(ch) => {
            let mut bytes = if chord.control {
                vec![control_byte(*ch)?]
            } else {
                utf8(*ch)
            };
            if chord.alt {
                bytes.insert(0, ESC);
            }
            Some(bytes)
        }
        KeyToken::Text(text) => {
            if text.is_empty() {
                return None;
            }
            let mut bytes = Vec::with_capacity(text.len() + usize::from(chord.alt));
            if chord.alt {
                bytes.push(ESC);
            }
            bytes.extend_from_slice(text.as_bytes());
            Some(bytes)
        }
        KeyToken::Named(key) => named(*key, chord, modes),
    }
}

fn named(key: NamedKey, chord: &KeyChord, modes: TerminalModes) -> Option<Vec<u8>> {
    match key {
        NamedKey::Enter => simple(b'\r', chord),
        NamedKey::Backspace => simple(DEL, chord),
        NamedKey::Escape => simple(ESC, chord),
        NamedKey::Tab if chord.shift => Some(b"\x1b[Z".to_vec()),
        NamedKey::BackTab => Some(b"\x1b[Z".to_vec()),
        NamedKey::Tab => simple(b'\t', chord),
        NamedKey::Up => cursor(b'A', chord, modes),
        NamedKey::Down => cursor(b'B', chord, modes),
        NamedKey::Right => cursor(b'C', chord, modes),
        NamedKey::Left => cursor(b'D', chord, modes),
        NamedKey::Home => cursor(b'H', chord, modes),
        NamedKey::End => cursor(b'F', chord, modes),
        NamedKey::Insert => tilde(2, chord),
        NamedKey::Delete => tilde(3, chord),
        NamedKey::PageUp => tilde(5, chord),
        NamedKey::PageDown => tilde(6, chord),
    }
}

fn simple(byte: u8, chord: &KeyChord) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(1 + usize::from(chord.alt));
    if chord.alt {
        bytes.push(ESC);
    }
    bytes.push(byte);
    Some(bytes)
}

fn cursor(final_byte: u8, chord: &KeyChord, modes: TerminalModes) -> Option<Vec<u8>> {
    if modified(chord) {
        Some(vec![
            ESC,
            b'[',
            b'1',
            b';',
            b'0' + modifier_code(chord),
            final_byte,
        ])
    } else if modes.app_cursor() {
        Some(vec![ESC, b'O', final_byte])
    } else {
        Some(vec![ESC, b'[', final_byte])
    }
}

fn tilde(number: u8, chord: &KeyChord) -> Option<Vec<u8>> {
    if modified(chord) {
        Some(vec![
            ESC,
            b'[',
            b'0' + number,
            b';',
            b'0' + modifier_code(chord),
            b'~',
        ])
    } else {
        Some(vec![ESC, b'[', b'0' + number, b'~'])
    }
}

fn modified(chord: &KeyChord) -> bool {
    chord.shift || chord.alt || chord.control
}

/// `1 + shift + 2*alt + 4*control`; the encoder only emits single digits.
fn modifier_code(chord: &KeyChord) -> u8 {
    1 + u8::from(chord.shift) + 2 * u8::from(chord.alt) + 4 * u8::from(chord.control)
}

fn control_byte(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some(ch as u8 & 0x1f),
        'A'..='Z' => Some((ch as u8 | 0x20) & 0x1f),
        '@' | ' ' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(DEL),
        _ => None,
    }
}

fn utf8(ch: char) -> Vec<u8> {
    let mut buffer = [0_u8; 4];
    ch.encode_utf8(&mut buffer).as_bytes().to_vec()
}

#[cfg(test)]
mod tests;
