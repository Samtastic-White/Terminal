//! Platform keystrokes to the domain key vocabulary. Only translation lives
//! here; byte encoding belongs to `domain::keymap`.

use gpui::Keystroke;

use crate::domain::keymap::{KeyChord, KeyToken, NamedKey};

/// Converts one platform keystroke, or returns `None` when the key has no
/// legacy terminal representation (function keys, composed control text).
pub(super) fn key_chord(keystroke: &Keystroke) -> Option<KeyChord> {
    let modifiers = keystroke.modifiers;
    let token = if let Some(named) = NamedKey::from_name(&keystroke.key) {
        KeyToken::Named(named)
    } else {
        // Control chords read the key itself: some layouts deliver the
        // already-translated control character in `key_char`, which would
        // shadow `Ctrl+<letter>`.
        let control_key = modifiers
            .control
            .then_some(keystroke.key.as_str())
            .filter(|key| key.len() == 1 && key.is_ascii());
        let text = control_key
            .or_else(|| {
                keystroke
                    .key_char
                    .as_deref()
                    .filter(|text| !text.is_empty())
            })
            .or_else(|| (keystroke.key.chars().count() == 1).then_some(keystroke.key.as_str()))?;
        if modifiers.control {
            let mut chars = text.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyToken::Char(ch)
        } else if text.chars().count() == 1 {
            KeyToken::Char(text.chars().next()?)
        } else {
            KeyToken::Text(text.to_owned())
        }
    };

    Some(KeyChord {
        token,
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
    })
}

#[cfg(test)]
mod tests {
    use gpui::Keystroke;

    use super::key_chord;
    use crate::domain::keymap::{KeyToken, TerminalModes, encode_key};

    #[test]
    fn control_chords_read_the_key_even_when_key_char_is_translated() {
        // Some layouts hand over the already-translated control character.
        // `Ctrl+V` must still resolve to `v` so it encodes literal-next.
        let keystroke = Keystroke::parse("ctrl-v->\u{16}").expect("valid keystroke");
        let chord = key_chord(&keystroke).expect("control chord");

        assert_eq!(chord.token, KeyToken::Char('v'));
        assert_eq!(
            encode_key(&chord, TerminalModes::default()),
            Some(vec![0x16])
        );
    }

    #[test]
    fn named_keys_keep_their_vocabulary() {
        let keystroke = Keystroke::parse("up").expect("valid keystroke");
        let chord = key_chord(&keystroke).expect("named chord");

        assert!(matches!(chord.token, KeyToken::Named(_)));
    }
}
