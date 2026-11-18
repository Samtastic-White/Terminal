//! Block service unit tests: lifecycle routing, budgets, eviction, live-row
//! gating, and time labels.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use super::*;
use crate::domain::block::{BlockId, BlockStatus, CopyKind};
use crate::domain::output::{Row, Span, TextStyle};
use crate::domain::semantic::{SemanticEvent, ShellPhase};
use crate::domain::terminal_surface::SurfaceItem;

struct Clock {
    base: Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
        }
    }

    fn at(&self, seconds: u64) -> Timestamp {
        Timestamp {
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
            mono: self.base + Duration::from_secs(seconds),
        }
    }
}

fn row(text: &str) -> SurfaceItem {
    SurfaceItem::Row(Row {
        spans: vec![Span {
            style: TextStyle::default(),
            text: text.into(),
        }],
    })
}

fn event(event: SemanticEvent) -> SurfaceItem {
    SurfaceItem::Semantic(event)
}

fn open_block(service: &mut BlockService, clock: &Clock, second: u64, command: Option<&str>) {
    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(second));
    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(second));
    if let Some(command) = command {
        service.handle_items(
            vec![event(SemanticEvent::CommandLine {
                text: command.into(),
            })],
            clock.at(second),
        );
    }
}

fn finish_block(service: &mut BlockService, clock: &Clock, second: u64, code: u32) {
    service.handle_items(
        vec![event(SemanticEvent::CommandFinished { exit_code: code })],
        clock.at(second),
    );
}

#[test]
fn full_lifecycle_produces_a_completed_block() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.handle_items(vec![row("user@host:~$ ")], clock.at(1));
    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(2));
    service.handle_items(
        vec![event(SemanticEvent::CommandLine {
            text: "echo hi".into(),
        })],
        clock.at(2),
    );
    service.handle_items(vec![row("hi")], clock.at(2));
    finish_block(&mut service, &clock, 3, 0);

    assert_eq!(service.blocks().len(), 1);
    let block = &service.blocks()[0];
    assert_eq!(block.command(), Some("echo hi"));
    assert_eq!(block.status(), BlockStatus::Succeeded);
    assert_eq!(block.copy(CopyKind::Output), "hi");
    assert_eq!(block.copy(CopyKind::Command), "echo hi");
    assert_eq!(block.copy(CopyKind::WholeBlock), "echo hi\nhi");
    assert_eq!(
        service.phase(),
        ShellPhase::Prompting { prompt_open: false }
    );
}

#[test]
fn feedback_rows_route_to_the_pending_submission() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("if true".into());
    service.handle_items(vec![row("syntax error")], clock.at(1));

    assert_eq!(service.feedback_rows().len(), 1);
    assert_eq!(service.pending_text(), Some("if true"));

    finish_block(&mut service, &clock, 2, 1);

    assert_eq!(service.take_restored(), Some("if true".into()));
    assert!(service.awaiting_start());
    assert_eq!(service.feedback_rows().len(), 1);
}

#[test]
fn rows_without_a_pending_submission_are_silenced() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.handle_items(vec![row("async notice")], clock.at(1));

    assert!(service.feedback_rows().is_empty());
    assert!(service.pending_text().is_none());
}

#[test]
fn block_opened_attaches_pending_text_before_the_command_line_marker() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("echo submitted".into());
    service.handle_items(vec![row("feedback")], clock.at(1));
    assert_eq!(service.feedback_rows().len(), 1);

    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(2));
    service.handle_items(
        vec![event(SemanticEvent::CommandLine {
            text: "echo other".into(),
        })],
        clock.at(2),
    );

    assert_eq!(service.blocks()[0].command(), Some("echo submitted"));
    assert!(service.pending_text().is_none());
    assert!(service.feedback_rows().is_empty());
}

#[test]
fn prompt_open_unlocks_after_the_pending_text_was_attached() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("echo hi".into());
    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(1));
    assert!(service.awaiting_start());

    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(2));
    finish_block(&mut service, &clock, 3, 0);
    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(4));

    assert!(!service.awaiting_start());
}

#[test]
fn a_redrawn_prompt_unlocks_after_a_failed_submission() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("bad syntax".into());
    service.handle_items(
        vec![event(SemanticEvent::CommandFinished { exit_code: 1 })],
        clock.at(1),
    );
    assert!(service.awaiting_start());
    assert_eq!(service.take_restored(), Some("bad syntax".into()));

    // The shell never left the prompt, so its new `A` is a duplicate and
    // produces no machine transition.
    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(2));

    assert!(!service.awaiting_start());
}

#[test]
fn abort_pending_returns_the_text_and_clears_feedback() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("broken".into());
    service.handle_items(vec![row("feedback")], clock.at(1));

    assert_eq!(service.abort_pending(), Some("broken".into()));
    assert!(!service.awaiting_start());
    assert!(service.feedback_rows().is_empty());
    assert!(service.pending_text().is_none());
}

#[test]
fn session_end_restores_pending_text_and_clears_feedback() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("pending command".into());
    service.handle_items(vec![row("feedback")], clock.at(1));

    service.session_ended(clock.at(2));

    assert!(service.is_closed());
    assert_eq!(service.take_restored(), Some("pending command".into()));
    assert!(service.feedback_rows().is_empty());
}

#[test]
fn the_feedback_ring_drops_the_oldest_row_at_its_cap() {
    let clock = Clock::new();
    let mut service = BlockService::new(BlockLimits {
        block_max_rows: 4,
        session_max_rows: 100,
        session_max_blocks: 8,
        feedback_max_rows: 2,
        silence_max_rows: 4,
    });

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.begin_submission("cmd".into());
    service.handle_items(vec![row("first"), row("second"), row("third")], clock.at(1));

    let texts: Vec<String> = service
        .feedback_rows()
        .iter()
        .map(|row| row.spans.iter().map(|span| span.text.as_str()).collect())
        .collect();
    assert_eq!(texts, ["second", "third"]);
}

#[test]
fn history_is_newest_first_and_collapses_consecutive_duplicates() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    for (index, command) in ["one", "one", "two"].iter().enumerate() {
        let second = index as u64 * 2;
        open_block(&mut service, &clock, second, Some(command));
        finish_block(&mut service, &clock, second + 1, 0);
    }
    open_block(&mut service, &clock, 6, Some("running"));

    assert_eq!(service.history_len(), 2);
    assert_eq!(service.history_entry(0), Some("two"));
    assert_eq!(service.history_entry(1), Some("one"));
    assert_eq!(service.history_entry(2), None);
}

#[test]
fn cwd_is_snapshotted_when_the_block_opens() {
    let clock = Clock::new();
    let mut service = BlockService::default();

    service.handle_items(
        vec![event(SemanticEvent::DirectoryChanged {
            path: PathBuf::from("/tmp"),
        })],
        clock.at(0),
    );
    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(0));
    service.handle_items(
        vec![event(SemanticEvent::DirectoryChanged {
            path: PathBuf::from("/var"),
        })],
        clock.at(1),
    );

    assert_eq!(service.blocks()[0].cwd(), Some(&PathBuf::from("/tmp")));
}

#[test]
fn only_the_first_command_line_payload_wins() {
    let clock = Clock::new();
    let mut service = BlockService::default();
    open_block(&mut service, &clock, 0, Some("echo first"));
    service.handle_items(
        vec![event(SemanticEvent::CommandLine {
            text: "echo second".into(),
        })],
        clock.at(0),
    );

    assert_eq!(service.blocks()[0].command(), Some("echo first"));
}

#[test]
fn session_end_interrupts_the_running_block_and_keeps_output() {
    let clock = Clock::new();
    let mut service = BlockService::default();
    open_block(&mut service, &clock, 0, Some("sleep 10"));
    service.handle_items(vec![row("partial")], clock.at(1));

    service.session_ended(clock.at(2));

    let block = &service.blocks()[0];
    assert_eq!(block.status(), BlockStatus::Interrupted);
    assert_eq!(block.copy(CopyKind::Output), "partial");
    assert_eq!(service.phase(), ShellPhase::Closed);
}

#[test]
fn command_start_while_running_interrupts_and_splits() {
    let clock = Clock::new();
    let mut service = BlockService::default();
    open_block(&mut service, &clock, 0, Some("first"));
    service.handle_items(vec![row("one")], clock.at(1));

    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(2));
    service.handle_items(vec![row("two")], clock.at(2));
    finish_block(&mut service, &clock, 3, 1);

    assert_eq!(service.blocks().len(), 2);
    assert_eq!(service.blocks()[0].status(), BlockStatus::Interrupted);
    assert_eq!(service.blocks()[0].copy(CopyKind::Output), "one");
    assert_eq!(
        service.blocks()[1].status(),
        BlockStatus::Failed { exit_code: 1 }
    );
    assert_eq!(service.blocks()[1].copy(CopyKind::Output), "two");
}

#[test]
fn session_rows_evict_completed_blocks_before_the_running_one() {
    let clock = Clock::new();
    let mut service = BlockService::new(BlockLimits {
        block_max_rows: 4,
        session_max_rows: 3,
        session_max_blocks: 8,
        feedback_max_rows: 4,
        silence_max_rows: 4,
    });

    open_block(&mut service, &clock, 0, Some("first"));
    service.handle_items(vec![row("a"), row("b")], clock.at(0));
    finish_block(&mut service, &clock, 1, 0);

    open_block(&mut service, &clock, 2, Some("second"));
    service.handle_items(vec![row("c"), row("d")], clock.at(2));

    assert_eq!(service.blocks().len(), 1);
    assert_eq!(service.blocks()[0].command(), Some("second"));
    assert_eq!(service.blocks()[0].copy(CopyKind::Output), "c\nd");
}

#[test]
fn the_running_block_is_never_evicted_by_the_session_row_budget() {
    let clock = Clock::new();
    let mut service = BlockService::new(BlockLimits {
        block_max_rows: 4,
        session_max_rows: 1,
        session_max_blocks: 8,
        feedback_max_rows: 4,
        silence_max_rows: 4,
    });

    open_block(&mut service, &clock, 0, Some("stream"));
    service.handle_items(vec![row("a"), row("b")], clock.at(0));

    assert_eq!(service.blocks().len(), 1);
    assert_eq!(service.blocks()[0].copy(CopyKind::Output), "a\nb");
}

#[test]
fn session_block_budget_evicts_the_oldest_completed_block() {
    let clock = Clock::new();
    let mut service = BlockService::new(BlockLimits {
        block_max_rows: 4,
        session_max_rows: 100,
        session_max_blocks: 2,
        feedback_max_rows: 4,
        silence_max_rows: 4,
    });

    for (index, command) in ["one", "two", "three"].iter().enumerate() {
        let second = index as u64 * 2;
        open_block(&mut service, &clock, second, Some(command));
        service.handle_items(vec![row(command)], clock.at(second));
        finish_block(&mut service, &clock, second + 1, 0);
    }

    assert_eq!(service.blocks().len(), 2);
    assert_eq!(service.blocks()[0].command(), Some("two"));
    assert_eq!(service.blocks()[1].command(), Some("three"));
}

#[test]
fn live_row_is_visible_only_while_running() {
    let clock = Clock::new();
    let mut service = BlockService::default();
    let live = || Some(Row::default());

    assert!(service.live_row(live()).is_none());

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(0));
    assert!(service.live_row(live()).is_none());

    service.handle_items(vec![event(SemanticEvent::CommandStart)], clock.at(1));
    assert!(service.live_row(live()).is_some());

    service.handle_items(
        vec![event(SemanticEvent::CommandFinished { exit_code: 0 })],
        clock.at(2),
    );
    assert!(service.live_row(live()).is_none());

    service.handle_items(vec![event(SemanticEvent::PromptStart)], clock.at(3));
    assert!(service.live_row(live()).is_none());

    service.session_ended(clock.at(4));
    assert!(service.live_row(live()).is_none());
}

#[test]
fn terminal_reset_closes_the_service_and_ignores_later_items() {
    let clock = Clock::new();
    let mut service = BlockService::default();
    open_block(&mut service, &clock, 0, Some("sleep 10"));
    service.handle_items(vec![row("partial")], clock.at(1));

    service.handle_items(vec![event(SemanticEvent::TerminalReset)], clock.at(2));
    service.handle_items(vec![row("after-reset")], clock.at(3));

    let block = &service.blocks()[0];
    assert_eq!(block.status(), BlockStatus::Interrupted);
    assert_eq!(block.copy(CopyKind::Output), "partial");
    assert_eq!(service.phase(), ShellPhase::Closed);
    assert!(service.live_row(Some(Row::default())).is_none());
}

#[test]
fn copy_block_returns_none_for_an_unknown_id() {
    let service = BlockService::default();

    assert!(
        service
            .copy_block(BlockId::new(99), CopyKind::Output)
            .is_none()
    );
}

#[test]
fn relative_labels_cover_each_window() {
    let start = SystemTime::UNIX_EPOCH;

    assert_eq!(
        format_relative(start, start + Duration::from_secs(59)),
        "just now"
    );
    assert_eq!(
        format_relative(start, start + Duration::from_secs(60)),
        "1 min ago"
    );
    assert_eq!(
        format_relative(start, start + Duration::from_secs(59 * 60)),
        "59 min ago"
    );
    assert_eq!(
        format_relative(start, start + Duration::from_secs(3_600)),
        "1 h ago"
    );
    assert_eq!(
        format_relative(start, start + Duration::from_secs(86_400)),
        "1 d ago"
    );
}

#[test]
fn duration_labels_cover_sub_minute_minutes_and_hours() {
    assert_eq!(format_duration(Duration::from_millis(0)), "0.00s");
    assert_eq!(format_duration(Duration::from_millis(420)), "0.42s");
    assert_eq!(format_duration(Duration::from_secs(65)), "1m 05s");
    assert_eq!(format_duration(Duration::from_secs(3_665)), "1h 01m");
}
