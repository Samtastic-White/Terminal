//! State machine unit tests: every phase/marker combination.

use std::path::PathBuf;

use super::*;

fn machine_in(phase: ShellPhase) -> BlockStateMachine {
    let mut machine = BlockStateMachine::new();
    machine.phase = phase;
    machine
}

#[test]
fn prompt_start_opens_the_prompt_window_from_cold() {
    let mut machine = BlockStateMachine::new();

    let effects = machine.on(SemanticEvent::PromptStart);

    assert_eq!(effects, vec![Transition::PromptOpened]);
    assert_eq!(machine.phase(), ShellPhase::Prompting { prompt_open: true });
}

#[test]
fn prompt_start_reopens_a_closed_prompt_window() {
    let mut machine = machine_in(ShellPhase::Prompting { prompt_open: false });

    let effects = machine.on(SemanticEvent::PromptStart);

    assert_eq!(effects, vec![Transition::PromptOpened]);
    assert_eq!(machine.phase(), ShellPhase::Prompting { prompt_open: true });
}

#[test]
fn duplicate_prompt_start_is_ignored() {
    let mut machine = machine_in(ShellPhase::Prompting { prompt_open: true });

    let effects = machine.on(SemanticEvent::PromptStart);

    assert!(effects.is_empty());
    assert_eq!(machine.phase(), ShellPhase::Prompting { prompt_open: true });
}

#[test]
fn prompt_start_while_running_is_ignored() {
    let mut machine = machine_in(ShellPhase::Running);

    let effects = machine.on(SemanticEvent::PromptStart);

    assert!(effects.is_empty());
    assert_eq!(machine.phase(), ShellPhase::Running);
}

#[test]
fn prompt_end_never_transitions() {
    for phase in [
        ShellPhase::Cold,
        ShellPhase::Prompting { prompt_open: true },
        ShellPhase::Prompting { prompt_open: false },
        ShellPhase::Running,
        ShellPhase::Closed,
    ] {
        let mut machine = machine_in(phase);

        let effects = machine.on(SemanticEvent::PromptEnd);

        assert!(effects.is_empty(), "unexpected effects in {phase:?}");
        assert_eq!(machine.phase(), phase);
    }
}

#[test]
fn command_start_opens_a_block_from_the_prompt() {
    let mut machine = machine_in(ShellPhase::Prompting { prompt_open: true });

    let effects = machine.on(SemanticEvent::CommandStart);

    assert_eq!(effects, vec![Transition::BlockOpened]);
    assert_eq!(machine.phase(), ShellPhase::Running);
}

#[test]
fn command_start_is_ignored_while_cold() {
    let mut machine = BlockStateMachine::new();

    let effects = machine.on(SemanticEvent::CommandStart);

    assert!(effects.is_empty());
    assert_eq!(machine.phase(), ShellPhase::Cold);
}

#[test]
fn command_start_while_running_interrupts_and_opens() {
    let mut machine = machine_in(ShellPhase::Running);

    let effects = machine.on(SemanticEvent::CommandStart);

    assert_eq!(
        effects,
        vec![Transition::BlockInterrupted, Transition::BlockOpened]
    );
    assert_eq!(machine.phase(), ShellPhase::Running);
}

#[test]
fn command_finished_completes_the_running_block() {
    let mut machine = machine_in(ShellPhase::Running);

    let effects = machine.on(SemanticEvent::CommandFinished { exit_code: 130 });

    assert_eq!(effects, vec![Transition::BlockCompleted { exit_code: 130 }]);
    assert_eq!(
        machine.phase(),
        ShellPhase::Prompting { prompt_open: false }
    );
}

#[test]
fn command_finished_without_a_running_block_is_ignored() {
    for phase in [
        ShellPhase::Cold,
        ShellPhase::Prompting { prompt_open: true },
        ShellPhase::Prompting { prompt_open: false },
    ] {
        let mut machine = machine_in(phase);

        let effects = machine.on(SemanticEvent::CommandFinished { exit_code: 0 });

        assert!(effects.is_empty(), "unexpected effects in {phase:?}");
        assert_eq!(machine.phase(), phase);
    }
}

#[test]
fn reset_interrupts_a_running_block_and_closes() {
    let mut machine = machine_in(ShellPhase::Running);

    let effects = machine.on(SemanticEvent::TerminalReset);

    assert_eq!(
        effects,
        vec![Transition::BlockInterrupted, Transition::Closed]
    );
    assert_eq!(machine.phase(), ShellPhase::Closed);
}

#[test]
fn reset_closes_without_interrupting_when_no_block_runs() {
    for phase in [
        ShellPhase::Cold,
        ShellPhase::Prompting { prompt_open: true },
        ShellPhase::Prompting { prompt_open: false },
    ] {
        let mut machine = machine_in(phase);

        let effects = machine.on(SemanticEvent::TerminalReset);

        assert_eq!(effects, vec![Transition::Closed], "phase {phase:?}");
        assert_eq!(machine.phase(), ShellPhase::Closed);
    }
}

#[test]
fn reset_while_closed_is_a_no_op() {
    let mut machine = machine_in(ShellPhase::Closed);

    let effects = machine.on(SemanticEvent::TerminalReset);

    assert!(effects.is_empty());
    assert_eq!(machine.phase(), ShellPhase::Closed);
}

#[test]
fn every_event_is_ignored_after_close() {
    let events = [
        SemanticEvent::PromptStart,
        SemanticEvent::PromptEnd,
        SemanticEvent::CommandStart,
        SemanticEvent::CommandFinished { exit_code: 0 },
        SemanticEvent::TerminalReset,
    ];

    for event in events {
        let mut machine = machine_in(ShellPhase::Closed);

        let effects = machine.on(event.clone());

        assert!(effects.is_empty(), "unexpected effects for {event:?}");
        assert_eq!(machine.phase(), ShellPhase::Closed);
    }
}

#[test]
fn payload_events_do_not_transition() {
    let mut machine = machine_in(ShellPhase::Prompting { prompt_open: true });

    let command = machine.on(SemanticEvent::CommandLine {
        text: "echo hi".into(),
    });
    let directory = machine.on(SemanticEvent::DirectoryChanged {
        path: PathBuf::from("/tmp"),
    });

    assert!(command.is_empty());
    assert!(directory.is_empty());
    assert_eq!(machine.phase(), ShellPhase::Prompting { prompt_open: true });
}
