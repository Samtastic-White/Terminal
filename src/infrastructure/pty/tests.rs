//! Adapter unit tests: round-trip, env contract, wrapper wiring, idempotent
//! spawn, backpressure, EOF, and failure mapping.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use portable_pty::CommandBuilder;

use super::ZSH_PATH;
use super::adapter::{PtyAdapter, apply_shell_env};
use crate::common::errors::TerminalError;
use crate::domain::pty::PtyPort;
use crate::domain::size::TerminalSize;

fn drain_until_received(
    receiver: &mut Receiver<Vec<u8>>,
    needle: &[u8],
    deadline: Duration,
) -> Option<Vec<u8>> {
    let start = std::time::Instant::now();
    let mut acc: Vec<u8> = Vec::new();
    while start.elapsed() < deadline {
        let chunk = match receiver.recv_timeout(deadline.saturating_sub(start.elapsed())) {
            Ok(bytes) => bytes,
            Err(_) => return None,
        };
        acc.extend(chunk);
        if acc.windows(needle.len()).any(|w| w == needle) {
            return Some(acc);
        }
    }
    None
}

#[test]
fn bytes_round_trip_through_spawned_shell() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();

    drain_until_received(&mut receiver, b" ", Duration::from_secs(5));

    adapter.write(b"printf 'NT_ROUNDTRIP_%s\\n' 42\r").unwrap();
    let acc = drain_until_received(&mut receiver, b"NT_ROUNDTRIP_42", Duration::from_secs(5))
        .expect("round-trip output within deadline");
    assert!(acc.windows(15).any(|w| w == b"NT_ROUNDTRIP_42"));
}

#[test]
fn child_env_contract_sets_identity_and_clears_nesting_guard() {
    use std::ffi::OsStr;

    let mut command = CommandBuilder::new(ZSH_PATH);
    command.env("NATIVE_TERMINAL_ACTIVE", "1");
    command.env("SHELL", "/bin/bash");
    apply_shell_env(&mut command);

    assert_eq!(command.get_env("NATIVE_TERMINAL"), Some(OsStr::new("1")));
    assert_eq!(command.get_env("TERM"), Some(OsStr::new("xterm-256color")));
    assert_eq!(command.get_env("COLORTERM"), Some(OsStr::new("truecolor")));
    assert_eq!(command.get_env("SHELL"), Some(OsStr::new(ZSH_PATH)));
    assert_eq!(command.get_env("NATIVE_TERMINAL_ACTIVE"), None);
}

#[test]
fn build_command_points_zdotdir_at_the_wrapper() {
    let mut adapter = PtyAdapter::new();
    let command = adapter.build_command().expect("command built");
    let integration = adapter.integration.as_ref().expect("integration prepared");

    assert_eq!(
        command.get_env("ZDOTDIR"),
        Some(integration.dir().as_os_str())
    );
    assert_eq!(
        command.get_env("NT_USER_ZDOTDIR"),
        Some(integration.user_zdotdir().as_os_str())
    );
}

#[test]
fn respawn_while_alive_keeps_the_original_session() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();

    adapter.spawn(TerminalSize::new(40, 12)).unwrap();
    adapter.write(b"printf 'NT_IDEMPOTENT_%s\\n' 7\r").unwrap();

    let acc = drain_until_received(&mut receiver, b"NT_IDEMPOTENT_7", Duration::from_secs(5))
        .expect("first channel must stay live after a redundant spawn");
    assert!(acc.windows(15).any(|w| w == b"NT_IDEMPOTENT_7"));
}

#[test]
fn flood_output_exceeds_channel_capacity_without_loss() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let mut receiver = adapter.take_output().unwrap();

    drain_until_received(&mut receiver, b" ", Duration::from_secs(5));
    adapter
        .write(
            b"dd if=/dev/zero bs=1024 count=1024 2>/dev/null | tr '\\0' x; printf 'NT_FLOOD_%s\\n' DONE\r",
        )
        .unwrap();

    // Leave the reader parked on a full channel; the child must throttle.
    std::thread::sleep(Duration::from_millis(300));

    let acc = drain_until_received(&mut receiver, b"NT_FLOOD_DONE", Duration::from_secs(20))
        .expect("flood output within deadline");
    assert!(
        acc.len() >= 1024 * 1024,
        "received {} bytes, expected the whole 1 MiB",
        acc.len()
    );
    assert!(acc.windows(13).any(|w| w == b"NT_FLOOD_DONE"));
}

#[test]
fn eof_marks_session_ended() {
    let mut adapter = PtyAdapter::new();
    adapter.spawn(TerminalSize::new(80, 24)).unwrap();
    let receiver = adapter.take_output().unwrap();

    adapter.write(b"exit\r").unwrap();
    let deadline = Duration::from_secs(10);
    let start = std::time::Instant::now();
    while receiver
        .recv_timeout(
            deadline
                .saturating_sub(start.elapsed())
                .max(Duration::from_millis(1)),
        )
        .is_ok()
    {}
    assert!(!adapter.is_alive(), "EOF must finish the session");
}

#[test]
fn spawn_failure_maps_to_terminal_error() {
    let mut adapter = PtyAdapter::with_shell("/nonexistent-shell-9f3ad1");
    let error = adapter
        .spawn(TerminalSize::new(80, 24))
        .expect_err("missing shell must fail");
    assert!(matches!(error, TerminalError::Pty(_)));
}
