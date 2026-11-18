//! Private-mode tracking: DECSET/DECRST `?1` (`APP_CURSOR`) and RIS.

use super::*;
use crate::domain::terminal_surface::TerminalSurfacePort;

fn modes_after(chunks: &[&[u8]]) -> crate::domain::keymap::TerminalModes {
    let mut writer = BlockTextWriter::new(80);
    for chunk in chunks {
        writer.write(chunk);
    }
    writer.modes()
}

#[test]
fn decset_app_cursor_sets_the_mode() {
    assert!(modes_after(&[b"\x1b[?1h"]).app_cursor());
}

#[test]
fn decrst_app_cursor_clears_the_mode() {
    assert!(!modes_after(&[b"\x1b[?1h", b"\x1b[?1l"]).app_cursor());
}

#[test]
fn mode_one_is_applied_whatever_its_parameter_position() {
    assert!(modes_after(&[b"\x1b[?1;2004h"]).app_cursor());
    assert!(modes_after(&[b"\x1b[?2004;1h"]).app_cursor());
}

#[test]
fn unrelated_private_modes_leave_app_cursor_alone() {
    assert!(!modes_after(&[b"\x1b[?25h", b"\x1b[?2004h"]).app_cursor());
}

#[test]
fn a_private_sequence_split_across_writes_still_applies() {
    let mut writer = BlockTextWriter::new(80);
    writer.write(b"\x1b[?1");
    assert!(!writer.modes().app_cursor());

    writer.write(b"h");
    assert!(writer.modes().app_cursor());
}

#[test]
fn ris_resets_modes_before_emitting_the_reset_event() {
    let mut writer = BlockTextWriter::new(80);
    writer.write(b"\x1b[?1h");
    assert!(writer.modes().app_cursor());

    writer.write(b"\x1bc");

    assert!(!writer.modes().app_cursor());
    assert!(matches!(
        writer.take_items().as_slice(),
        [SurfaceItem::Semantic(SemanticEvent::TerminalReset)]
    ));
}

#[test]
fn non_private_csi_with_intermediates_is_ignored() {
    // DECSCUSR (`CSI 1 SP q`) carries an intermediate and changes no mode.
    let mut writer = BlockTextWriter::new(80);
    writer.write(b"\x1b[?1h");
    writer.write(b"\x1b[1 q");

    assert!(writer.modes().app_cursor());
    assert!(writer.take_items().is_empty());
}
