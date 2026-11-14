//! Adapter body: child command construction, spawn with reader thread, port
//! implementation, and teardown.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtyPair, native_pty_system};
use tracing::{debug, info, warn};

use super::{CHANNEL_CAP, PTY_BUFFER_BYTES, ZSH_PATH};
use crate::common::errors::{Result, TerminalError};
use crate::domain::pty::PtyPort;
use crate::domain::size::TerminalSize;

use super::shell_integration::ShellIntegration;

fn pty_err<E: std::fmt::Display>(error: E) -> TerminalError {
    TerminalError::Pty(error.to_string())
}

/// The live PTY session. One window spawns exactly one adapter.
pub struct PtyAdapter {
    shell_path: &'static str,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    /// Shared with the reader thread: it must reap the child after EOF.
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    /// False once the reader finished (EOF or channel closed).
    reader_alive: Arc<AtomicBool>,
    spawned: bool,
    /// Handed to the composition root (channel consumer) after spawn.
    pending_receiver: Option<Receiver<Vec<u8>>>,
    /// Wrapper dotfiles injected for the current spawn; removed on drop.
    pub(super) integration: Option<ShellIntegration>,
}

impl Default for PtyAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyAdapter {
    /// Adapter for `/bin/zsh`; the shell environment carries
    /// `NATIVE_TERMINAL=1`.
    pub fn new() -> Self {
        Self::with_shell(ZSH_PATH)
    }

    /// Adapter with an explicit shell path (tests: failure path and
    /// integration runs).
    pub fn with_shell(shell_path: &'static str) -> Self {
        Self {
            shell_path,
            master: None,
            writer: None,
            child: Arc::new(Mutex::new(None)),
            killer: None,
            reader_alive: Arc::new(AtomicBool::new(true)),
            spawned: false,
            pending_receiver: None,
            integration: None,
        }
    }

    /// Take the output channel receiver (one handoff; None afterwards).
    pub fn take_output(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.pending_receiver.take()
    }

    pub(super) fn build_command(&mut self) -> Result<CommandBuilder> {
        let mut command = CommandBuilder::new(self.shell_path);
        // Interactive login shell: zsh reads its own startup files.
        command.arg("-il");
        apply_shell_env(&mut command);

        let home = std::env::var_os("HOME").map(PathBuf::from);
        let user_zdotdir = std::env::var_os("ZDOTDIR")
            .map(PathBuf::from)
            .or(home)
            .ok_or_else(|| TerminalError::Pty("HOME is not set".into()))?;
        let integration = ShellIntegration::prepare(&user_zdotdir)?;
        command.env("ZDOTDIR", integration.dir());
        command.env("NT_USER_ZDOTDIR", integration.user_zdotdir());
        debug!("shell integration prepared");
        self.integration = Some(integration);

        Ok(command)
    }
}

/// Child environment contract: the engine owns terminal identity and shell,
/// and clears the nesting guard inherited from any parent session.
pub(super) fn apply_shell_env(command: &mut CommandBuilder) {
    command.env("NATIVE_TERMINAL", "1");
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("SHELL", ZSH_PATH);
    command.env_remove("NATIVE_TERMINAL_ACTIVE");
}

impl PtyPort for PtyAdapter {
    /// Spawns the shell in a new PTY sized to `size`, starts the reader
    /// thread, and stashes the receiver for `take_output`. Idempotent while a
    /// live session exists.
    fn spawn(&mut self, size: TerminalSize) -> Result<()> {
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

        let command = self.build_command()?;
        let child = slave.spawn_command(command).map_err(pty_err)?;
        let killer = child.clone_killer();
        let reader = master.try_clone_reader().map_err(pty_err)?;
        let writer = master.take_writer().map_err(pty_err)?;

        // Blocking reads are legal here: this is never the render thread.
        // `send` blocks on a full bounded channel, so the kernel pipe
        // throttles the child.
        let (sender, receiver) = sync_channel::<Vec<u8>>(CHANNEL_CAP);
        let reaper = self.child.clone();
        let reader_alive = self.reader_alive.clone();

        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut reader = reader;
                let mut buffer = vec![0u8; PTY_BUFFER_BYTES];
                debug!("pty reader started");
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break, // EOF (child death / PTY closed)
                        Ok(n) => {
                            let started = Instant::now();
                            if sender.send(buffer[..n].to_vec()).is_err() {
                                break; // consumer gone; nothing to deliver
                            }
                            let waited = started.elapsed();
                            if waited >= Duration::from_millis(1) {
                                debug!(
                                    wait_us = waited.as_micros() as u64,
                                    capacity = CHANNEL_CAP,
                                    "pty reader blocked on full channel"
                                );
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
                info!("pty reader finished");
                drop(sender);
                reader_alive.store(false, Ordering::Relaxed);
                // Reap outside the lock: `wait()` can block on the child.
                let child = reaper.lock().ok().and_then(|mut guard| guard.take());
                if let Some(mut child) = child {
                    let _ = child.wait();
                }
            })
            .map_err(pty_err)?;

        if let Ok(mut guard) = self.child.lock() {
            *guard = Some(child);
        }
        self.master = Some(master);
        self.writer = Some(writer);
        self.killer = Some(killer);
        self.pending_receiver = Some(receiver);
        self.spawned = true;
        info!(
            shell = self.shell_path,
            cols = size.cols,
            rows = size.rows,
            "pty spawned"
        );
        Ok(())
    }

    /// Completes short writes before returning.
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let Some(writer) = self.writer.as_mut() else {
            return Err(TerminalError::Pty("session not spawned".into()));
        };
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| {
                warn!(error = %error, "pty write failed");
                pty_err(error)
            })
    }

    /// Immediate resize; the kernel SIGWINCH reaches the foreground group.
    /// Callers must not debounce.
    fn resize(&mut self, size: TerminalSize) -> Result<()> {
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
            .map_err(pty_err)?;
        debug!(cols = size.cols, rows = size.rows, "pty resized");
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.reader_alive.load(Ordering::Relaxed)
    }
}

impl Drop for PtyAdapter {
    fn drop(&mut self) {
        // Kill before closing the PTY: an interactive shell may survive the
        // master close and keep the reader blocked in `wait`.
        if let Some(mut killer) = self.killer.take() {
            let _ = killer.kill();
        }
        self.writer.take();
        self.master.take();
        self.reader_alive.store(false, Ordering::Relaxed);
    }
}
