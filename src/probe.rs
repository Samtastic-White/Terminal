// Idle/pacing probe, enabled with `NT_MEASURE`. Probe state lives in statics so
// the frame callback and the sampling thread never touch the app's own state.

use std::io::{BufRead as _, BufReader};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const PROBE_SETTLE: Duration = Duration::from_secs(15);
const PROBE_SAMPLES: usize = 10;
const PROBE_STEP: Duration = Duration::from_secs(2);

/// Absolute PSS ceiling from the roadmap; the ~70 MB platform floor (GPU
/// driver stack + gpui engine) is the reference point, not a budget.
const IDLE_PSS_CEILING_KB: u64 = 80_000;
const IDLE_PSS_FLOOR_KB: u64 = 70_000;
/// Vsync period of a 144 Hz monitor, in microseconds (informational target).
const PACING_TARGET_US: u64 = 6_944;
/// Fewer idle gaps than this cannot describe a cadence.
const MIN_PACING_SAMPLES: usize = 10;

static TOTAL_FRAMES: AtomicU64 = AtomicU64::new(0);
static FRAME_GAPS_US: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static LAST_FRAME: Mutex<Option<Instant>> = Mutex::new(None);
static PROBE_ENABLED: OnceLock<bool> = OnceLock::new();

pub(crate) fn measure_enabled() -> bool {
    measure_enabled_from(std::env::var("NT_MEASURE").ok().as_deref())
}

pub(crate) fn measure_enabled_from(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Called once per rendered frame. No-op unless the probe is active.
pub(crate) fn record_frame() {
    if !*PROBE_ENABLED.get_or_init(measure_enabled) {
        return;
    }
    TOTAL_FRAMES.fetch_add(1, Ordering::Relaxed);
    let now = Instant::now();
    let mut last = LAST_FRAME.lock().unwrap();
    if let Some(previous) = *last {
        FRAME_GAPS_US
            .lock()
            .unwrap()
            .push(now.duration_since(previous).as_micros() as u64);
    }
    *last = Some(now);
}

pub(crate) fn backend_name() -> &'static str {
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

pub(crate) fn run_probe(backend: &'static str) {
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
    let frames = TOTAL_FRAMES.load(Ordering::Relaxed);
    // An idle window that renders almost nothing yields no cadence to judge;
    // reporting PASS there would be vacuous.
    let paced = frames > 0 && sorted.len() >= MIN_PACING_SAMPLES && p50 > 0;
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
        frames, p50, p99, max, hz, dropped
    );
    println!(
        "VERDICT idle PSS: {} (p50 last 5 samples = {} kB, ceiling {} kB, reference floor {} kB) | VERDICT pacing: {} (gaps = {}, P99 {} us vs observed tolerance {} us; expected {} us @ 144 Hz; dropped = {})",
        if idle_pss < IDLE_PSS_CEILING_KB {
            "PASS"
        } else {
            "FAIL"
        },
        idle_pss,
        IDLE_PSS_CEILING_KB,
        IDLE_PSS_FLOOR_KB,
        if !paced {
            "NO DATA"
        } else if p99 <= p99_tolerance && dropped == 0 {
            "PASS"
        } else {
            "FAIL"
        },
        sorted.len(),
        p99,
        p99_tolerance,
        PACING_TARGET_US,
        dropped
    );
    println!("[PROBE done — close the window when you are ready]");
}
