//! Header time formatting: wall/mono timestamps, relative age, and duration
//! labels.

use std::time::{Duration, Instant, SystemTime};

/// Wall-clock instant paired with a monotonic instant for duration math.
#[derive(Debug, Clone, Copy)]
pub struct Timestamp {
    pub wall: SystemTime,
    pub mono: Instant,
}

impl Timestamp {
    pub fn now() -> Self {
        Self {
            wall: SystemTime::now(),
            mono: Instant::now(),
        }
    }
}

/// Short relative label for a block header.
pub fn format_relative(start: SystemTime, now: SystemTime) -> String {
    let elapsed = now.duration_since(start).unwrap_or_default().as_secs();
    if elapsed < 60 {
        "just now".into()
    } else if elapsed < 3_600 {
        format!("{} min ago", elapsed / 60)
    } else if elapsed < 86_400 {
        format!("{} h ago", elapsed / 3_600)
    } else {
        format!("{} d ago", elapsed / 86_400)
    }
}

/// Header duration label: sub-second precision below a minute, then minutes
/// and hours.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.2}s")
    } else if seconds < 3_600.0 {
        let total = duration.as_secs();
        format!("{}m {:02}s", total / 60, total % 60)
    } else {
        let total = duration.as_secs();
        format!("{}h {:02}m", total / 3_600, (total % 3_600) / 60)
    }
}
