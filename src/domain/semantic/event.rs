//! A shell integration marker decoded from the terminal stream.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticEvent {
    /// OSC `133;A`: a new prompt is being drawn.
    PromptStart,
    /// OSC `133;B`: the prompt ended and user input begins.
    PromptEnd,
    /// OSC `133;C`: the command was submitted.
    CommandStart,
    /// OSC `133;D;<code>`: the command finished with a shell status code.
    CommandFinished { exit_code: u32 },
    /// OSC `633;E;<command>`: the exact command line.
    CommandLine { text: String },
    /// OSC `633;P;Cwd=<path>`: the shell's current directory.
    DirectoryChanged { path: PathBuf },
    /// ESC `c`: the terminal was reset.
    TerminalReset,
}
