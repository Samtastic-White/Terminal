//! `src/infrastructure/pty/` — `PtyAdapter` implements the Domain
//! `PtyPort` with `portable-pty` (design.md § 2 contracts, § 3 flow).
//!
//! Threading contract (spec BR-2/BR-3): reads run on a dedicated OS
//! thread; chunks travel through a bounded `SyncSender` whose `send`
//! blocks when full — the reader parks and the kernel pipe buffer
//! throttles the child. **PTY bytes are never dropped.** EOF marks the
//! session ended and reaps the child (no zombies — design § 8).

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};

use portable_pty::{Child, CommandBuilder, MasterPty, PtyPair, native_pty_system};

use crate::common::errors::{Result, TerminalError};
use crate::domain::pty::{PtyPort, PtySize};

/// Reader chunk size (design § 4).
const PTY_BUFFER_BYTES: usize = 8 * 1024;
/// Bounded channel capacity in chunks (design § 4). ≈ 512 KiB in flight.
const CHANNEL_CAP: usize = 64;
/// Production child shell (spec BR-1).
const ZSH_PATH: &str = "/bin/zsh";

/// `TerminalError::Pty` conversion helper for portable-pty failures.
fn pty_err<E: std::fmt::Display>(error: E) -> TerminalError {
    TerminalError::Pty(error.to_string())
}

/// The live PTY session. One window spawns exactly one adapter (BR-1).
pub struct PtyAdapter {
    shell_path: &'static str,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    /// Shared with the reader thread: it must reap the child after EOF.
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    /// False once the reader finished (EOF or channel closed).
    reader_alive: Arc<AtomicBool>,
    spawned: bool,
    /// Handed to the composition root (channel consumer) after spawn.
    pending_receiver: Option<Receiver<Vec<u8>>>,
}

impl Default for PtyAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyAdapter {
    /// Adapter for `/bin/zsh`, engine-gated from this phase on
    /// (`NATIVE_TERMINAL=1` so the Phase 2 Zsh hooks can activate later
    /// without a protocol change).
    pub fn new() -> Self {
        Self::with_shell(ZSH_PATH)
    }

    /// explicit shell path (tests: failure-path and integration runs).
    pub fn with_shell(shell_path: &'static str) -> Self {
        Self {
            shell_path,
            master: None,
            writer: None,
            child: Arc::new(Mutex::new(None)),
            reader_alive: Arc::new(AtomicBool::new(true)),
            spawned: false,
            pending_receiver: None,
        }
    }

    /// Take the output channel receiver (one handoff; None afterwards).
    pub fn take_output(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.pending_receiver.take()
    }

    fn build_command(&self) -> CommandBuilder {
        let mut command = CommandBuilder::new(self.shell_path);
        // Interactive login shell: zsh reads its own startup files.
        command.arg("-il");
        command.env("NATIVE_TERMINAL", "1");
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command
    }
}

impl PtyPort for PtyAdapter {
    /// Spawn `/bin/zsh` in a new PTY sized to `size`, start the reader
    /// thread, and stash the receiver for `take_output`. Idempotent while
    /// a live session exists (BR-1).
    fn spawn(&mut self, size: PtySize) -> Result<()> {
        if self.spawned && self.is_alive() {
            return Ok(());
        }

        let pair = native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(pty_err)?;
        let PtyPair { master, slave } = pair;

        let child = slave.spawn_command(self.build_command()).map_err(pty_err)?;
        let reader = master.try_clone_reader().map_err(pty_err)?;
        let writer = master.take_writer().map_err(pty_err)?;

        // Reader worker (BR-2): blocking reads are legal here — this is
        // never the render thread. `send` blocks on a full bounded
        // channel (BR-3): the kernel pipe then throttles the child.
        let (sender, receiver) = sync_channel::<Vec<u8>>(CHANNEL_CAP);
        let reaper = self.child.clone();
        let reader_alive = self.reader_alive.clone();

        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut reader = reader;
                let mut buffer = vec![0u8; PTY_BUFFER_BYTES];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break, // EOF (child death / PTY closed)
                        Ok(n) => {
                            if sender.send(buffer[..n].to_vec()).is_err() {
                                break; // consumer gone; nothing to deliver
                            }
                        }
                        Err(error) => {
                            if error.kind() == std::io::ErrorKind::Interrupted {
                                continue; // transient; not EOF
                            }
                            break;
                        }
                    }
                }
                drop(sender);
                reader_alive.store(false, Ordering::Relaxed);
                // Reap the child: no zombie (design § 8).
                if let Ok(mut guard) = reaper.lock()
                    && let Some(mut child) = guard.take()
                {
                    let _ = child.wait();
                }
            })
            .map_err(pty_err)?;

        if let Ok(mut guard) = self.child.lock() {
            *guard = Some(child);
        }
        self.master = Some(master);
        self.writer = Some(writer);
        self.pending_receiver = Some(receiver);
        self.spawned = true;
        Ok(())
    }

    /// Complete short writes before returning (design § 2 contract).
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let Some(writer) = self.writer.as_mut() else {
            return Err(TerminalError::Pty("session not spawned".into()));
        };
        writer.write_all(bytes).map_err(pty_err)?;
        writer.flush().map_err(pty_err)
    }

    /// Immediate resize; the kernel SIGWINCH reaches the foreground
    /// group (BR-7 — callers must not debounce).
    fn resize(&mut self, size: PtySize) -> Result<()> {
        let Some(master) = self.master.as_ref() else {
            return Err(TerminalError::Pty("session not spawned".into()));
        };
        master
            .resize(portable_pty::PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(pty_err)
    }

    fn is_alive(&self) -> bool {
        self.reader_alive.load(Ordering::Relaxed)
    }
}

impl Drop for PtyAdapter {
    fn drop(&mut self) {
        // Kill the child so its pipe closes and the reader finalizes;
        // the master drop closes our side of the PTY.
        self.writer.take();
        self.master.take();
        if let Some(mut child) = self.child.lock().ok().and_then(|mut guard| guard.take()) {
            let _ = child.kill();
        }
        self.reader_alive.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
        adapter.spawn(PtySize::new(80, 24)).unwrap();
        let mut receiver = adapter.take_output().unwrap();

        // Drain the banner noise until the prompt settles.
        drain_until_received(&mut receiver, b" ", Duration::from_secs(5));

        adapter.write(b"echo NT_ROUNDTRIP_42\r").unwrap();
        let acc = drain_until_received(&mut receiver, b"NT_ROUNDTRIP_42", Duration::from_secs(5))
            .expect("round-trip output within deadline");
        assert!(acc.windows(15).any(|w| w == b"NT_ROUNDTRIP_42"));
    }

    #[test]
    fn full_channel_blocks_the_reader_no_bytes_dropped() {
        // The windpark/backpressure seam (design § 7): a full sync channel
        // must park `send`, then release on drain.
        let (sender, receiver) = sync_channel::<Vec<u8>>(1);
        sender.send(vec![1]).unwrap();

        let parked_done = Arc::new(AtomicBool::new(false));
        let done_flag = parked_done.clone();
        let handle = std::thread::spawn(move || {
            sender.send(vec![2]).unwrap(); // blocks: channel full
            done_flag.store(true, Ordering::Relaxed);
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !parked_done.load(Ordering::Relaxed),
            "send must park while full"
        );

        let drained = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(drained, vec![1]);
        handle.join().unwrap();
        // The parked chunk lands after the drain — no bytes dropped.
        let parked = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(parked, vec![2]);
        assert!(parked_done.load(Ordering::Relaxed));
    }

    #[test]
    fn eof_marks_session_ended() {
        let mut adapter = PtyAdapter::new();
        adapter.spawn(PtySize::new(80, 24)).unwrap();
        let receiver = adapter.take_output().unwrap();

        adapter.write(b"exit\r").unwrap();
        let deadline = Duration::from_secs(10);
        let start = std::time::Instant::now();
        // Drain until the reader thread closes the channel.
        while receiver
            .recv_timeout(
                deadline
                    .saturating_sub(start.elapsed())
                    .max(Duration::from_millis(1)),
            )
            .is_ok()
        {}
        assert!(!adapter.is_alive(), "EOF must finish the session (BR-9)");
    }

    #[test]
    fn spawn_failure_maps_to_terminal_error() {
        let mut adapter = PtyAdapter::with_shell("/nonexistent-shell-9f3ad1");
        let error = adapter
            .spawn(PtySize::new(80, 24))
            .expect_err("missing shell must fail");
        assert!(matches!(error, TerminalError::Pty(_)));
    }
}
