//! Lifecycle position of the shell and the tolerant OSC 133 phase machine.

use super::event::SemanticEvent;

/// Lifecycle position of the shell relative to command blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellPhase {
    /// No prompt has been observed yet.
    #[default]
    Cold,
    /// A prompt is visible; `prompt_open` is true between A and C.
    Prompting { prompt_open: bool },
    /// A command is executing and owns the output stream.
    Running,
    /// The shell ended or the terminal was reset.
    Closed,
}

/// Effect the application must apply when the machine advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// A genuinely new prompt started; the prompt buffer must be cleared.
    PromptOpened,
    /// A command block started; the previous output stream is over.
    BlockOpened,
    /// The running block completed with the shell's exit code.
    BlockCompleted { exit_code: u32 },
    /// The running block ended without a `D` marker.
    BlockInterrupted,
    /// The shell will not produce more output.
    Closed,
}

/// Tolerant OSC 133 phase machine.
///
/// Markers that do not fit the current phase are ignored rather than
/// reported, matching the shell protocol's lossy delivery.
#[derive(Debug, Clone, Default)]
pub struct BlockStateMachine {
    pub(super) phase: ShellPhase,
}

impl BlockStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> ShellPhase {
        self.phase
    }

    /// Applies an event and returns the effects, in application order.
    pub fn on(&mut self, event: SemanticEvent) -> Vec<Transition> {
        match event {
            SemanticEvent::PromptStart => self.on_prompt_start(),
            SemanticEvent::CommandStart => self.on_command_start(),
            SemanticEvent::CommandFinished { exit_code } => self.on_command_finished(exit_code),
            SemanticEvent::TerminalReset => self.on_terminal_reset(),
            SemanticEvent::PromptEnd
            | SemanticEvent::CommandLine { .. }
            | SemanticEvent::DirectoryChanged { .. } => Vec::new(),
        }
    }

    fn on_prompt_start(&mut self) -> Vec<Transition> {
        match self.phase {
            ShellPhase::Cold | ShellPhase::Prompting { prompt_open: false } => {
                self.phase = ShellPhase::Prompting { prompt_open: true };
                vec![Transition::PromptOpened]
            }
            // A duplicate A while Prompting{true} is a prompt redraw; a prompt
            // marker while Running belongs to the next prompt and is ignored.
            ShellPhase::Prompting { prompt_open: true }
            | ShellPhase::Running
            | ShellPhase::Closed => Vec::new(),
        }
    }

    fn on_command_start(&mut self) -> Vec<Transition> {
        match self.phase {
            ShellPhase::Prompting { .. } => {
                self.phase = ShellPhase::Running;
                vec![Transition::BlockOpened]
            }
            // A second C without D means the shell started a nested command
            // before reporting completion: close the open block and continue.
            ShellPhase::Running => vec![Transition::BlockInterrupted, Transition::BlockOpened],
            // Cold means integration loaded mid-session; there is no prompt
            // buffer to promote, so the output stays in the silence window.
            ShellPhase::Cold | ShellPhase::Closed => Vec::new(),
        }
    }

    fn on_command_finished(&mut self, exit_code: u32) -> Vec<Transition> {
        match self.phase {
            ShellPhase::Running => {
                self.phase = ShellPhase::Prompting { prompt_open: false };
                vec![Transition::BlockCompleted { exit_code }]
            }
            ShellPhase::Cold | ShellPhase::Prompting { .. } | ShellPhase::Closed => Vec::new(),
        }
    }

    fn on_terminal_reset(&mut self) -> Vec<Transition> {
        match self.phase {
            ShellPhase::Running => {
                self.phase = ShellPhase::Closed;
                vec![Transition::BlockInterrupted, Transition::Closed]
            }
            ShellPhase::Cold | ShellPhase::Prompting { .. } => {
                self.phase = ShellPhase::Closed;
                vec![Transition::Closed]
            }
            ShellPhase::Closed => Vec::new(),
        }
    }
}
