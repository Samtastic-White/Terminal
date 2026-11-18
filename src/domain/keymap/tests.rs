//! Key-encoding unit tests: every row of the byte table.

use super::*;

fn named(key: NamedKey) -> KeyChord {
    KeyChord {
        token: KeyToken::Named(key),
        shift: false,
        alt: false,
        control: false,
    }
}

fn char_chord(ch: char) -> KeyChord {
    KeyChord {
        token: KeyToken::Char(ch),
        shift: false,
        alt: false,
        control: false,
    }
}

fn text_chord(text: &str) -> KeyChord {
    KeyChord {
        token: KeyToken::Text(text.to_owned()),
        shift: false,
        alt: false,
        control: false,
    }
}

fn encode(chord: &KeyChord) -> Vec<u8> {
    encode_key(chord, TerminalModes::default()).expect("chord encodes")
}

fn encode_in(chord: &KeyChord, modes: TerminalModes) -> Vec<u8> {
    encode_key(chord, modes).expect("chord encodes")
}

#[test]
fn tracked_modes_start_empty_and_reset() {
    let mut modes = TerminalModes::default();
    assert!(!modes.app_cursor());

    modes.set_app_cursor(true);
    assert!(modes.app_cursor());

    modes.reset();
    assert!(!modes.app_cursor());
}

#[test]
fn control_letters_map_to_their_control_byte() {
    for (letter, byte) in [
        ('a', 0x01),
        ('c', 0x03),
        ('d', 0x04),
        ('v', 0x16),
        ('z', 0x1a),
    ] {
        let mut chord = char_chord(letter);
        chord.control = true;
        assert_eq!(encode(&chord), vec![byte]);
    }
}

#[test]
fn control_is_case_insensitive() {
    let mut lower = char_chord('a');
    lower.control = true;
    let mut upper = char_chord('A');
    upper.control = true;
    upper.shift = true;

    assert_eq!(encode(&upper), encode(&lower));
}

#[test]
fn control_space_and_at_map_to_nul() {
    for ch in [' ', '@'] {
        let mut chord = char_chord(ch);
        chord.control = true;
        assert_eq!(encode(&chord), vec![0x00]);
    }
}

#[test]
fn the_control_punctuation_family_follows_the_same_rule() {
    for (punctuation, byte) in [
        ('[', 0x1b),
        ('\\', 0x1c),
        (']', 0x1d),
        ('^', 0x1e),
        ('_', 0x1f),
    ] {
        let mut chord = char_chord(punctuation);
        chord.control = true;
        assert_eq!(encode(&chord), vec![byte]);
    }
}

#[test]
fn control_question_mark_is_del() {
    let mut chord = char_chord('?');
    chord.control = true;

    assert_eq!(encode(&chord), vec![0x7f]);
}

#[test]
fn a_control_chord_without_a_legacy_byte_is_dropped() {
    let mut chord = char_chord('1');
    chord.control = true;

    assert_eq!(encode_key(&chord, TerminalModes::default()), None);
}

#[test]
fn alt_prefixes_the_encoded_bytes() {
    let mut chord = char_chord('x');
    chord.alt = true;
    assert_eq!(encode(&chord), b"\x1bx".to_vec());

    let mut chord = text_chord("é");
    chord.alt = true;
    let mut expected = vec![0x1b];
    expected.extend_from_slice(b"\xc3\xa9");
    assert_eq!(encode(&chord), expected);
}

#[test]
fn alt_control_keeps_one_escape_prefix() {
    let mut chord = char_chord('c');
    chord.alt = true;
    chord.control = true;

    assert_eq!(encode(&chord), vec![0x1b, 0x03]);
}

#[test]
fn printable_text_is_its_utf8_bytes() {
    assert_eq!(encode(&char_chord('a')), b"a".to_vec());
    assert_eq!(encode(&char_chord('é')), b"\xc3\xa9".to_vec());
    assert_eq!(encode(&char_chord('漢')), b"\xe6\xbc\xa2".to_vec());
    assert_eq!(
        encode(&text_chord("e\u{301}")),
        "e\u{301}".as_bytes().to_vec()
    );
}

#[test]
fn empty_composed_text_is_dropped() {
    assert_eq!(encode_key(&text_chord(""), TerminalModes::default()), None);
}

#[test]
fn simple_named_keys_use_their_legacy_byte() {
    assert_eq!(encode(&named(NamedKey::Enter)), vec![b'\r']);
    assert_eq!(encode(&named(NamedKey::Backspace)), vec![0x7f]);
    assert_eq!(encode(&named(NamedKey::Tab)), vec![b'\t']);
    assert_eq!(encode(&named(NamedKey::Escape)), vec![0x1b]);
}

#[test]
fn shift_tab_is_csi_z() {
    let mut chord = named(NamedKey::Tab);
    chord.shift = true;
    assert_eq!(encode(&chord), b"\x1b[Z".to_vec());
    assert_eq!(encode(&named(NamedKey::BackTab)), b"\x1b[Z".to_vec());
}

#[test]
fn plain_arrows_use_csi_without_app_cursor() {
    for (key, final_byte) in [
        (NamedKey::Up, b'A'),
        (NamedKey::Down, b'B'),
        (NamedKey::Right, b'C'),
        (NamedKey::Left, b'D'),
    ] {
        assert_eq!(encode(&named(key)), vec![0x1b, b'[', final_byte]);
    }
}

#[test]
fn plain_arrows_use_ss3_under_app_cursor() {
    let mut modes = TerminalModes::default();
    modes.set_app_cursor(true);

    for (key, final_byte) in [
        (NamedKey::Up, b'A'),
        (NamedKey::Down, b'B'),
        (NamedKey::Right, b'C'),
        (NamedKey::Left, b'D'),
    ] {
        assert_eq!(encode_in(&named(key), modes), vec![0x1b, b'O', final_byte]);
    }
}

#[test]
fn home_and_end_switch_dialect_with_app_cursor() {
    assert_eq!(encode(&named(NamedKey::Home)), b"\x1b[H".to_vec());
    assert_eq!(encode(&named(NamedKey::End)), b"\x1b[F".to_vec());

    let mut modes = TerminalModes::default();
    modes.set_app_cursor(true);
    assert_eq!(encode_in(&named(NamedKey::Home), modes), b"\x1bOH".to_vec());
    assert_eq!(encode_in(&named(NamedKey::End), modes), b"\x1bOF".to_vec());
}

#[test]
fn modified_arrows_use_the_modifier_code_table() {
    let cases = [
        (true, false, false, 2),
        (false, true, false, 3),
        (true, true, false, 4),
        (false, false, true, 5),
        (true, false, true, 6),
        (false, true, true, 7),
        (true, true, true, 8),
    ];
    for (shift, alt, control, code) in cases {
        let chord = KeyChord {
            token: KeyToken::Named(NamedKey::Up),
            shift,
            alt,
            control,
        };
        let expected = format!("\x1b[1;{code}A").into_bytes();
        assert_eq!(encode(&chord), expected, "code {code}");
    }
}

#[test]
fn modified_home_and_end_ignore_app_cursor() {
    let mut modes = TerminalModes::default();
    modes.set_app_cursor(true);
    let chord = KeyChord {
        token: KeyToken::Named(NamedKey::Home),
        shift: false,
        alt: false,
        control: true,
    };

    assert_eq!(encode_in(&chord, modes), b"\x1b[1;5H".to_vec());
}

#[test]
fn unmodified_paging_keys_use_tilde_forms() {
    assert_eq!(encode(&named(NamedKey::Insert)), b"\x1b[2~".to_vec());
    assert_eq!(encode(&named(NamedKey::Delete)), b"\x1b[3~".to_vec());
    assert_eq!(encode(&named(NamedKey::PageUp)), b"\x1b[5~".to_vec());
    assert_eq!(encode(&named(NamedKey::PageDown)), b"\x1b[6~".to_vec());
}

#[test]
fn modified_paging_keys_insert_the_modifier_code() {
    let chord = KeyChord {
        token: KeyToken::Named(NamedKey::PageDown),
        shift: true,
        alt: true,
        control: false,
    };

    assert_eq!(encode(&chord), b"\x1b[6;4~".to_vec());
}

#[test]
fn alt_on_simple_named_keys_is_a_single_escape_prefix() {
    let mut chord = named(NamedKey::Enter);
    chord.alt = true;
    assert_eq!(encode(&chord), vec![0x1b, b'\r']);

    let chord = KeyChord {
        token: KeyToken::Named(NamedKey::PageUp),
        shift: false,
        alt: true,
        control: false,
    };
    assert_eq!(encode(&chord), b"\x1b[5;3~".to_vec());
}

#[test]
fn every_named_key_converts_from_its_platform_name() {
    for (name, key) in [
        ("enter", NamedKey::Enter),
        ("backspace", NamedKey::Backspace),
        ("tab", NamedKey::Tab),
        ("backtab", NamedKey::BackTab),
        ("escape", NamedKey::Escape),
        ("up", NamedKey::Up),
        ("down", NamedKey::Down),
        ("left", NamedKey::Left),
        ("right", NamedKey::Right),
        ("home", NamedKey::Home),
        ("end", NamedKey::End),
        ("pageup", NamedKey::PageUp),
        ("pagedown", NamedKey::PageDown),
        ("insert", NamedKey::Insert),
        ("delete", NamedKey::Delete),
    ] {
        assert_eq!(NamedKey::from_name(name), Some(key), "{name}");
    }
}

#[test]
fn function_and_unknown_keys_have_no_conversion() {
    for name in ["f1", "f12", "media", "", "F5"] {
        assert_eq!(NamedKey::from_name(name), None, "{name}");
    }
}
