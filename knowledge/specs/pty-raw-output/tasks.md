# Tasks — PTY Subsystem and Raw Output

> Phase 1 contract. Implements against the spec + design (Gates 2 approved;
> this file is Gate 3). Ordered by dependency; each task carries its own
> acceptance criterion. Rust code is written and verified inside the
> container (`mise run shell`); standard verification (`cargo check`,
> `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo test`) applies
> to every task from T2 onward (AGENTS.md § 8).

## T1 — Domain: ports and output primitives

Create `src/domain/pty.rs` (`PtyPort` trait, `PtySize {cols, rows}` with
sane clamps) and `src/domain/output.rs` (`TextStyle {fg, bg, attrs}`,
`Span {style, text}` with column widths, `Row`), plus `src/common/errors.rs`
(`TerminalError` with `Pty`, `Channel`, `Shell` variants) per design § 1–2.
Wire module tree (`src/domain.rs`/`mod` files; `main.rs` untouched except
module declarations).

**AC:** `cargo test` passes headless; `src/domain/` imports nothing beyond
`std` + the project's own modules; types carry `///` doc comments.

## T2 — Parser: writer core (text, C0, pending-wrap)

Create `src/infrastructure/parser/mod.rs`: `BlockTextWriter` wrapping
`vte::Parser` implementing `vte::Perform` for BR-5's text/C0 subset
(printable `char`s, `CR`, `LF`, `BS`, `HT` with `TAB_STOP`, `BEL` ignored,
pending-wrap auto-wrap). Col-advance via `unicode_width`. Add `vte` +
`unicode-width` to `Cargo.toml` (approved scope, design § 9).

**AC:** unit tests: plain text rows; `CRLF` breaks rows; bare `LF` moves
down only (staircase by design); `BS` clamps at column 0; `HT` snaps to
multiples of 8 and clips at width; a character at the last column stays until
the next printable (deferred wrap; no fabricated empty rows).

## T3 — Parser: SGR, erase, style attributes

Extend the writer with SGR (0/1/3/4/22/23/24, 30–37/90–97 + 39,
40–47/100–107 + 49, `38;5;n`, `38;2;r;g;b` and bg twins), and `EL`(0/1/2)/
`ED`(0/1/2) on the current logical row (erase paints with the active
background at erase time; `ED 3` ignored). Unsupported CSI/OSC/DCS
families: consumed silently (BR-5/BR-6) with a Phase-4-pointer comment
(design § 10).

**AC:** unit tests: each SGR class applied from the next char and reset on
`0`; EL/ED modes erase the correct region with active bg; `\x1b[999;999H`
and an unknown OSC never print, never panic.

## T4 — Parser: golden fixtures (the 7 families)

Add the headless golden harness (design § 7): fixtures as Rust constants,
one named test per family — CJK/emoji widths, SGR palette, wrap at last
column, `\r` + `\x1b[K` rewrites, EL/ED, UTF-8 multibyte split across two
chunk feeds, and line-cap eviction order.

**AC:** all seven tests green; each fixture asserts full `Row` content
(text + style), byte inputs written as raw `b"…"` constants (no ad-hoc ANSI
construction in assertions).

## T5 — Application: session document + ingest

Create `src/application/session.rs`: `SessionDocument` (`VecDeque<Row>`
capped `OUTPUT_MAX_ROWS`, drop-oldest, newest retained; rows keep their
creation-time wrap width) and `SessionService` (`spawn_session`,
`ingest_bytes`, `write_keystrokes`, `resize`, `is_alive`, document read
access) — gpui-free per design § 2.

**AC:** unit tests (headless): feeding > cap lines evicts the oldest and
retains the newest; `ingest_bytes` applies writer output; service calls
delegate to the injected `PtyPort` (test double, no real PTY).

## T6 — Infrastructure: PtyAdapter + reader worker

Create `src/infrastructure/pty/mod.rs`: implement `PtyPort` with
`portable-pty` (native openpty, `/bin/zsh` interactive login, child env
`NATIVE_TERMINAL=1`, `TERM=xterm-256color`, `COLORTERM=truecolor`; short
writes completed); blocking reader OS thread feeding a bounded
`SyncSender` (`CHANNEL_CAP`); EOF/death finalization (design § 2, § 8).
Add `portable-pty` to `Cargo.toml`.

**AC:** unit tests with a `portable_pty` headless pair (no GPUI): bytes
spawned→read round-trip; a full `SyncSender` blocks `send` and wakes on
drain; EOF marks `is_alive() == false` without panic; error paths map to
`TerminalError::Pty`.

## T7 — Presentation: render surface (styled rows, tail-follow)

Create `src/presentation/theme.rs` (16/256→RGB palette fallback for
128–255? no: theme resolves 16-color table + 256/24-bit passthrough;
monospace font + row-height metric) and `src/presentation/session_view.rs`:
render the document rows as styled text, auto-follow while the viewport is
docked to the bottom (BR-10), manual scroll lookup suspends follow.

**AC:** manual: window shows the streaming prompt and colored output
(`ls --color=always`, ANSI demo with 24-bit colors) with correct styles;
scrolling up keeps position, returning to the end resumes tail-follow.
Code never parses escape bytes (conventions § Shell Protocol Hygiene).

## T8 — Presentation + Application: keyboard passthrough

Implement the key→bytes table (design § 5) in Presentation, routed via
`SessionService::write_keystrokes` → `PtyPort::write`; unrepresentable
keys are dropped silently; paste feeds the same line-by-line path
(bracketed paste unsupported, spec OQ-3 conservative resolution).

**AC:** manual: typing executes commands; arrows navigate zsh history;
`Ctrl+C` interrupts a running output flood; `Ctrl+D` ends the session
cleanly (window stays open, no panic); no direct Presentation→PTY call
exists in the dependency graph.

## T9 — Resize wiring

Wire window-resize → recompute cols/rows from content size ÷ row metrics →
`SessionService::resize` → `PtyPort::resize` (immediate, no debounce) +
writer wrap-width update (BR-7, design § 6).

**AC:** manual: resizing the window makes the next shell prompt/line wrap
at the new width immediately (SIGWINCH); rows stored before the resize
keep their original wrapping; `Resize` handler adds no debounce timer.

## T10 — Bootstrap integration + E2E checklist

Wire composition root in `src/main.rs` (adapters → services → view),
keeping the Phase 0 measurement probe untouched. Run the manual E2E
checklist: prompt visible; colors; CJK width; progress-bar case; keyboard
case (T8); resize case (T9); flood check (`cat /dev/urandom` ~30 s: window
interactive, RSS flat, memory bounded).

**AC:** checklist executed with results recorded in the session log; PTY
bytes never dropped (pre-writer) verified by the bounded/parking path;
no regression of NT_MEASURE output.

## T11 — Phase-close verification + roadmap tick

Full standard verification in the container; run the provenance guard
(`rg -i "denver technologies|license-alacritty|warp_terminal|waveterm" src/`
must be empty); verify every Phase 1 roadmap criterion against the spec's
AC list; tick `knowledge/roadmap.md` § Phase 1 checkboxes (fill the
verification note with repro commands), link the spec directory.

**AC:** `cargo check && cargo clippy -- -D warnings && cargo fmt --check
&& cargo test` all green; provenance guard empty; roadmap Phase 1 fully
ticked or explicitly deferred items documented.
