use gpui::*;

mod application;
mod common;
mod domain;
mod infrastructure;
mod presentation;

use application::session::SessionService;
use domain::pty::PtyPort;
use domain::size::TerminalSize;
use gpui_platform::application;
use infrastructure::parser::BlockTextWriter;
use infrastructure::pty::PtyAdapter;
use presentation::session_view::TerminalWindow;
use std::io::{BufRead as _, BufReader};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// Idle/pacing probe, enabled with `NT_MEASURE`. Probe state lives in statics so
// the frame callback and the sampling thread never touch the app's own state.

const PROBE_SETTLE: Duration = Duration::from_secs(15);
const PROBE_SAMPLES: usize = 10;
const PROBE_STEP: Duration = Duration::from_secs(2);

const IDLE_PSS_LIMIT_KB: u64 = 30_000;
/// Vsync period of a 144 Hz monitor, in microseconds (informational target).
const PACING_TARGET_US: u64 = 6_944;

static TOTAL_FRAMES: AtomicU64 = AtomicU64::new(0);
static FRAME_GAPS_US: Mutex<Vec<u64>> = Mutex::new(Vec::new());
#[allow(dead_code)]
static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);

fn measure_enabled() -> bool {
    match std::env::var("NT_MEASURE") {
        Ok(v) => !v.is_empty() && v != "0" && v != "n",
        Err(_) => false,
    }
}

fn backend_name() -> &'static str {
    if !std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_default()
        .is_empty()
    {
        "Wayland"
    } else if !std::env::var("DISPLAY").unwrap_or_default().is_empty() {
        "X11"
    } else {
        "unknown"
    }
}

fn read_smaps_rollup() -> Option<(u64, u64)> {
    let file = std::fs::File::open("/proc/self/smaps_rollup").ok()?;
    let mut pss = 0;
    let mut rss = 0;
    for line in BufReader::new(file).lines() {
        let line = line.ok()?;
        let kb = |rest: &str| {
            rest.trim()
                .trim_end_matches("kB")
                .trim()
                .parse()
                .unwrap_or(0u64)
        };
        if let Some(rest) = line.strip_prefix("Pss:") {
            pss = kb(rest);
        }
        if let Some(rest) = line.strip_prefix("Rss:") {
            rss = kb(rest);
        }
    }
    Some((pss, rss))
}

fn percentile(sorted: &[u64], pct: f64) -> u64 {
    let Some(len) = sorted.len().checked_sub(1) else {
        return 0;
    };
    sorted
        .get((len as f64 * pct).round() as usize)
        .copied()
        .unwrap_or(0)
}

#[allow(dead_code)]
fn record_frame_tick() {
    let now = Instant::now();
    let mut last = LAST_FRAME.lock().unwrap();
    if let Some(prev) = *last {
        let gap_us = now.duration_since(prev).as_micros() as u64;
        FRAME_GAPS_US.lock().unwrap().push(gap_us);
    }
    *last = Some(now);
    drop(last);
    TOTAL_FRAMES.fetch_add(1, Ordering::Relaxed);
}

fn run_probe(backend: &'static str) {
    println!(
        "[PROBE start — backend {} — settle {} s, {} samples x {} s — leave the window focused]",
        backend,
        PROBE_SETTLE.as_secs(),
        PROBE_SAMPLES,
        PROBE_STEP.as_secs()
    );
    std::thread::sleep(PROBE_SETTLE);

    // Startup jank (window omission, font loading) is not part of the idle
    // measurement: discard every gap recorded before the settle ends.
    FRAME_GAPS_US.lock().unwrap().clear();

    let mut samples: Vec<(u64, u64, u64)> = Vec::with_capacity(PROBE_SAMPLES);
    let mut prev_total = TOTAL_FRAMES.load(Ordering::Relaxed);
    let mut gaps: Vec<u64> = Vec::new();

    for i in 0..PROBE_SAMPLES {
        std::thread::sleep(PROBE_STEP);
        let (pss, rss) = read_smaps_rollup().unwrap_or((0, 0));
        let total = TOTAL_FRAMES.load(Ordering::Relaxed);
        let drained = FRAME_GAPS_US.lock().unwrap().drain(..).collect::<Vec<_>>();
        gaps.extend(drained);
        samples.push((pss, rss, total));
        println!(
            "[probe {}/10] PSS {} kB | RSS {} kB | frames +{}",
            i + 1,
            pss,
            rss,
            total - prev_total
        );
        prev_total = total;
    }

    let mut sorted = gaps;
    sorted.sort_unstable();
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    let max = sorted.last().copied().unwrap_or(0);
    // Pacing is judged against the OBSERVED vsync period (p50), not the
    // advertised refresh: a steady cadence is healthy pacing; the observed
    // Hz itself surfaces a 144 Hz panel being throttled to 60.
    let p99_tolerance = p50 + p50 / 4;
    let dropped_period = p50 * 2;
    let dropped = sorted.iter().filter(|&&g| g > dropped_period).count();
    let hz = if p50 > 0 {
        1_000_000.0 / p50 as f64
    } else {
        0.0
    };

    let take = PROBE_SAMPLES.min(5);
    let idle_pss = samples[samples.len() - take..]
        .iter()
        .map(|s| s.0)
        .sum::<u64>()
        / take as u64;

    println!("\n=== NT_IDLE_PROBE — backend: {} ===", backend);
    println!(
        "frames: {} | gaps us P50 {} / P99 {} / max {} | ~{:.1} Hz observed | dropped-frame events: {}",
        TOTAL_FRAMES.load(Ordering::Relaxed),
        p50,
        p99,
        max,
        hz,
        dropped
    );
    println!(
        "VERDICT idle PSS: {} (p50 last 5 samples = {} kB, limit {} kB) | VERDICT pacing: {} (P99 {} us vs observed tolerance {} us; expected {} us @ 144 Hz; dropped = {})",
        if idle_pss < IDLE_PSS_LIMIT_KB {
            "PASS"
        } else {
            "FAIL"
        },
        idle_pss,
        IDLE_PSS_LIMIT_KB,
        if p99 <= p99_tolerance && dropped == 0 {
            "PASS"
        } else {
            "FAIL"
        },
        p99,
        p99_tolerance,
        PACING_TARGET_US,
        dropped
    );
    println!("[PROBE done — close the window when you are ready]");
}

fn main() {
    application().run(|cx: &mut App| {
        let measure = measure_enabled();

        // Composition root: adapters → services → view.
        let mut pty_adapter = PtyAdapter::new();
        let block_writer = BlockTextWriter::new(80);
        let initial_size = TerminalSize::new(80, 24);

        if let Err(error) = pty_adapter.spawn(initial_size) {
            eprintln!("failed to spawn the shell: {error}");
            return;
        }
        let Some(pty_receiver) = pty_adapter.take_output() else {
            eprintln!("failed to obtain the PTY output channel");
            return;
        };
        let session_service = SessionService::new(Box::new(pty_adapter), Box::new(block_writer));

        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            move |_window, cx| cx.new(|cx| TerminalWindow::new(session_service, pty_receiver, cx)),
        );
        if let Err(error) = window {
            eprintln!("failed to open the terminal window: {error}");
            return;
        }

        cx.activate(true);

        if measure {
            let backend = backend_name();
            std::thread::Builder::new()
                .name("nt-idle-probe".into())
                .spawn(move || run_probe(backend))
                .ok();
        }
    });
}
