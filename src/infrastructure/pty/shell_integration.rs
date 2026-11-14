//! Zsh startup-file wrapper that injects the semantic markers without
//! touching the user's dotfiles.
//!
//! The child shell starts with `ZDOTDIR` pointed at a private temporary
//! directory. Each wrapper restores the user's `ZDOTDIR`, sources the user's
//! same startup file, records any `ZDOTDIR` change, then points `ZDOTDIR` back
//! at the wrapper so zsh finds the next file there.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{debug, warn};

use crate::common::errors::{Result, TerminalError};

#[cfg(test)]
mod tests;

const INTEGRATION_FILE: &str = "integration.zsh";
const INTEGRATION_SCRIPT: &str = include_str!("zsh_integration.zsh");
const WRAPPER_FILES: [(&str, bool); 5] = [
    (".zshenv", false),
    (".zprofile", false),
    (".zshrc", true),
    (".zlogin", false),
    (".zlogout", false),
];

static NEXT_WRAPPER_ID: AtomicU64 = AtomicU64::new(0);

/// A prepared wrapper directory; dropping it removes every generated file.
pub struct ShellIntegration {
    dir: PathBuf,
    user_zdotdir: PathBuf,
}

impl ShellIntegration {
    /// Writes the wrapper startup files for `user_zdotdir`.
    pub fn prepare(user_zdotdir: &Path) -> Result<Self> {
        let dir = create_wrapper_dir()?;
        let integration = Self {
            dir,
            user_zdotdir: user_zdotdir.to_path_buf(),
        };

        for (name, sources_integration) in WRAPPER_FILES {
            let path = integration.dir.join(name);
            write_private(&path, &wrapper_script(name, sources_integration))?;
        }
        write_private(&integration.dir.join(INTEGRATION_FILE), INTEGRATION_SCRIPT)?;

        Ok(integration)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn user_zdotdir(&self) -> &Path {
        &self.user_zdotdir
    }
}

impl Drop for ShellIntegration {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.dir) {
            debug!(error = %error, "shell integration cleanup failed");
        }
    }
}

fn create_wrapper_dir() -> Result<PathBuf> {
    let base = std::env::temp_dir();
    loop {
        let id = NEXT_WRAPPER_ID.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("native-terminal-{}-{id}", std::process::id()));
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(pty_error(error)),
        }
    }
}

/// Directory creation uses the process umask, so 0o700 is the widest mode.
fn write_private(path: &Path, contents: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(pty_error)?;
    file.write_all(contents.as_bytes()).map_err(pty_error)?;
    // Some filesystems apply a broader mode than requested; enforce it.
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(pty_error)?;
    Ok(())
}

fn wrapper_script(name: &str, sources_integration: bool) -> String {
    let integration = if sources_integration {
        "source \"$_nt_wrapper_zdotdir/integration.zsh\"\n"
    } else {
        ""
    };
    format!(
        "_nt_wrapper_zdotdir=${{ZDOTDIR:-$HOME}}\n\
         ZDOTDIR=${{NT_USER_ZDOTDIR:-$HOME}}\n\
         [[ -f \"$ZDOTDIR/{name}\" ]] && source \"$ZDOTDIR/{name}\"\n\
         NT_USER_ZDOTDIR=${{ZDOTDIR:-$NT_USER_ZDOTDIR}}\n\
         export NT_USER_ZDOTDIR\n\
         {integration}\
         ZDOTDIR=$_nt_wrapper_zdotdir\n\
         export ZDOTDIR\n\
         unset _nt_wrapper_zdotdir\n"
    )
}

fn pty_error(error: std::io::Error) -> TerminalError {
    warn!(error = %error, "shell integration preparation failed");
    TerminalError::Pty(error.to_string())
}
