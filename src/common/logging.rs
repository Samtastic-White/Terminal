//! Process-wide diagnostics setup.
//!
//! stderr is the only sink: stdout belongs to the measurement probe and the
//! user's terminal. The default filter is `warn`, so a normal run is silent;
//! `NT_LOG` (fallback `RUST_LOG`) overrides it. Formatting is pretty only on a
//! TTY, which keeps redirected output plain and parseable.

use std::io::{IsTerminal as _, stderr};
use std::sync::Once;

use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

const DEFAULT_FILTER: &str = "warn";

static INIT: Once = Once::new();

/// Installs the global subscriber once; later calls are no-ops.
///
/// A malformed filter degrades to `warn` instead of disabling diagnostics.
/// Failure to install (a subscriber already set by an embedding host) is
/// reported on stderr without aborting the process.
pub fn init() {
    INIT.call_once(|| {
        let directive = log_directive();
        let (targets, parse_error) = parse_filter(&directive);
        let pretty = pretty_output(
            stderr().is_terminal(),
            std::env::var_os("NO_COLOR").as_deref(),
        );

        let installed = if pretty {
            tracing_subscriber::registry()
                .with(
                    fmt::layer()
                        .pretty()
                        .with_ansi(true)
                        .with_writer(stderr)
                        .with_filter(targets),
                )
                .try_init()
        } else {
            tracing_subscriber::registry()
                .with(
                    fmt::layer()
                        .with_ansi(false)
                        .with_writer(stderr)
                        .with_filter(targets),
                )
                .try_init()
        };

        match installed {
            Ok(()) => {
                if let Some(error) = parse_error {
                    tracing::warn!(
                        directive = %directive,
                        error = %error,
                        "invalid log filter; using warn"
                    );
                }
                tracing::info!(filter = %directive, "logging initialized");
            }
            Err(error) => {
                eprintln!("failed to install the tracing subscriber: {error}");
            }
        }
    });
}

fn log_directive() -> String {
    let nt_log = std::env::var("NT_LOG").ok();
    let rust_log = std::env::var("RUST_LOG").ok();
    resolve_directive(nt_log.as_deref(), rust_log.as_deref())
}

fn resolve_directive(nt_log: Option<&str>, rust_log: Option<&str>) -> String {
    nt_log
        .filter(|value| !value.trim().is_empty())
        .or_else(|| rust_log.filter(|value| !value.trim().is_empty()))
        .unwrap_or(DEFAULT_FILTER)
        .to_string()
}

fn pretty_output(is_tty: bool, no_color: Option<&std::ffi::OsStr>) -> bool {
    is_tty && no_color.is_none_or(|value| value.is_empty())
}

fn parse_filter(directive: &str) -> (Targets, Option<String>) {
    match directive.parse::<Targets>() {
        Ok(targets) => (targets, None),
        Err(error) => (
            Targets::new().with_default(LevelFilter::WARN),
            Some(error.to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn nt_log_wins_over_rust_log() {
        assert_eq!(resolve_directive(Some("debug"), Some("trace")), "debug");
    }

    #[test]
    fn blank_nt_log_falls_back_to_rust_log() {
        assert_eq!(resolve_directive(Some("  "), Some("trace")), "trace");
    }

    #[test]
    fn empty_directives_fall_back_to_warn() {
        assert_eq!(resolve_directive(None, None), DEFAULT_FILTER);
        assert_eq!(resolve_directive(Some(""), Some("")), DEFAULT_FILTER);
    }

    #[test]
    fn documented_filter_syntax_parses() {
        let (_targets, error) = parse_filter("native_terminal=debug,infrastructure::pty=trace");
        assert!(error.is_none());
    }

    #[test]
    fn invalid_directive_falls_back_to_warn() {
        let (targets, error) = parse_filter("native_terminal=notalevel");
        assert!(error.is_some());
        assert_eq!(targets, Targets::new().with_default(LevelFilter::WARN));
    }

    #[test]
    fn empty_no_color_keeps_pretty_output() {
        assert!(pretty_output(true, None));
        assert!(pretty_output(true, Some(OsStr::new(""))));
        assert!(!pretty_output(true, Some(OsStr::new("1"))));
        assert!(!pretty_output(false, None));
    }
}
