# Spec — PTY Subsystem and Raw Output

> Capability directory: `knowledge/specs/pty-raw-output/`.
> Phase 1 of `knowledge/roadmap.md`. Decision 2026-11-07 (dual engine)
> governs this phase: `vte` + a bounded `Perform` implementation interpret
> normal-mode output now; the full grid engine is adopted in Phase 4 behind
> a Domain port. Source code is the implementation source of truth for how.

## Purpose

Connect the first real terminal behavior: spawn `/bin/zsh`, forward minimal
keyboard input, read raw child output on a background worker with real
backpressure, and render that output with bounded memory and correct ANSI
styling. This capability is the substrate every later phase consumes —
Phase 2 classifies its byte stream into semantic blocks, Phase 3 replaces
its keyboard passthrough with a native editor, Phase 4 swaps its writer
subset for a grid engine, Phase 6 virtualizes its bounded output.

## Capability Overview

Two halves, one subsystem:

1. **PTY subsystem.** The engine owns a PTY running an interactive `/bin/zsh`
   (engine-gated: `NATIVE_TERMINAL=1` present in the child environment from
   this phase on, so Phase 2 can activate Zsh hooks without a protocol
   change). Reading, writing, and resizing go through the Domain port
   `PtyPort` (spawn/write/resize contract); the reader runs exclusively on a
   background worker and delivers bytes to the Application layer through a
   bounded channel with real backpressure.
2. **Raw output interpretation.** The `BlockTextWriter` — a bounded
   `vte::Perform` implementation in the Infrastructure parser layer —
   consumes the byte stream and produces styled text rows. Its supported
   subset is defined by this spec (Business Rule 5). Output lands in a
   bounded session document and is rendered by the window while streaming.

## User Stories

- As a user, I want to run commands in zsh and see their raw output in the
  window, so that the terminal is finally usable.
- As a user, I want to type/paste/edit commands with the shell's own line
  editing (arrows, Ctrl+C, Ctrl+D, Ctrl+L), so that interacting with zsh
  works until the native editor arrives in Phase 3.
- As a user, I want colors and styles emitted by the shell and its tools to
  render correctly, so that output stays readable.
- As a user, I want the window to stay responsive under endless output
  (`cat /dev/urandom`, streaming logs), so that the UI never freezes and
  memory never grows without bound.
- As a user, I want window resize to reach running programs immediately,
  so that newly wrapped output matches the pane width.

## Business Rules

**BR-1 — Single shell, engine spawn.** The engine spawns exactly one
interactive `/bin/zsh` per window, with `NATIVE_TERMINAL=1` in the child
environment and the PTY sized to the window's content area. The shell's
own prompt/echo remain visible (ZLE untouched this phase) — no ghost
prompt is drawn by the engine.

**BR-2 — Never block the render thread.** PTY reads happen on background
infrastructure workers only. The GPUI render thread never performs a PTY
read, a filesystem query, or a child-process wait (AGENTS.md § 9;
architecture Common Mistake #1).

**BR-3 — Bounded channel with real backpressure.** The PTY→UI channel is
bounded. When it fills, the reader parks and the kernel pipe buffer
throttles the child process. **PTY bytes are never dropped before the
writer**: every byte is consumed by the parser; only interpreted content
(after the emulator) may be evicted by the line-cap policy (BR-4).

**BR-4 — Bounded session document.** Output never accumulates in an
unbounded `String`/`Vec` (Common Mistakes #4/#5). In this phase the whole
session is one bounded document of styled rows; when the line cap is
reached the **oldest completed rows are evicted (drop-oldest) and the
newest content is always retained**. The cap evicts interpreted lines
only, never raw bytes. Exact cap counts are pinned in `design.md`; the
policy shape is this rule. When Phase 2 introduces blocks, the same cap
policy moves from session scope to per-block scope.

**BR-5 — Writer subset (the exact contract).** The writer supports exactly
this and nothing else; anything outside it is consumed silently and never
printed (BR-6):

- **Text:** UTF-8 multibyte sequences reassemble correctly even when split
  across read chunks; each character advances the cursor by its Unicode
  width (`unicode-width`: CJK/emoji are 2 columns, everything else 1).
- **C0 controls:** `CR` (cursor to column 0), `LF` (one row down, column
  unchanged — plain `\n` staircases by design; shells emit `CRLF`),
  `BS` (one column left, floor 0), `HT` (advance to the next multiple-of-8
  tab stop, clipped at width), `BEL` and other C0 (ignored).
- **Auto-wrap (deferred):** a character printed in the last column stays
  in place; the next printed character starts the following row — VT100
  pending-wrap semantics, so repeated writes at the last column never
  fabricate empty rows.
- **SGR:** reset (0), bold (1), italic (3), underline (4) and their
  attribute-off forms (22/23/24); foreground/background 16-color
  (30–37/90–97, 40–47/100–107 + 39/49 defaults), 256-color (`38;5;n`
  / `48;5;n`), and 24-bit color (`38;2;r;g;b` / `48;2;r;g;b`).
- **Erase:** `EL` (`CSI K`, modes 0/1/2) and `ED` (`CSI J`, modes 0/1/2)
  acting on the current logical row only (this writer has no grid);
  erase paints with the background of the active style at the moment of
  the erase. `ED 3` (scrollback clear) is ignored.
- **Cursor tracking:** the current row's column (plus pending-wrap state)
  is tracked — this is exactly the `\r`-rewrite machinery progress bars
  need. Any vertical cursor movement is treated as committing the current
  row; explicit CUP/CUC/CUD family is outside the subset.
- **Ignored families (consumed, never printed):** all unsupported CSI,
  `OSC`/`DCS`/`APC`/`SOS`/`PM` (OSC 133 markers from Phase 2 arrivals are
  absorbed harmlessly), device status/cursor-position queries are recorded
  but **no responses are synthesized this phase** (Open Question 4).

**BR-6 — No crash on unknown input.** Unsupported, malformed, or truncated
sequences never panic, never crash, never render as mojibake; the writer
degrades deterministically.

**BR-7 — Resize semantics.** A window resize is forwarded to
`PtyPort::resize(cols, rows)` **immediately, with no debounce**; the
kernel delivers `SIGWINCH` to the zsh foreground group. Newly arriving
bytes wrap at the new width. Already-stored rows are **not re-wrapped
retroactively** (reflow is a Phase 4 non-inherited behavior); rows carry
the wrap width they were created with.

**BR-8 — Keyboard passthrough is routed, not leaked.** Minimal input is
encoded and written to the PTY through the Application layer use case;
Presentation never touches the PTY (architecture Common Mistake #3).
Supported keys: printable ASCII/UTF-8 text, `Enter` (`CR`), `Backspace`
(`DEL`), `Ctrl+C`, `Ctrl+D`, `Ctrl+L`, and arrow keys as plain `CSI`
sequences. Keys whose encoding is mode-dependent (SS3 vs CSI, kitty
protocol) are out of Phase 1 scope — the Phase 3 key-encoding layer owns
them.

**BR-9 — Shell death is not a crash.** Child exit, crash, or PTY EOF
finalizes the read loop and marks the session ended; the window remains
open showing the last content. A running notion of "interrupted block"
belongs to Phase 2 — nothing here panics or deadlocks on EOF.

**BR-10 — Rendering follows the tail while docked.** The window renders
the styled document continuously; while the viewport is at the bottom it
auto-follows new rows, and looking back up suspends the follow until the
user returns to the end.

## Edge Cases & Error Handling

- **Infinite output (memory budget).** Under continuous streams the
  bounded channel parks the reader (kernel throttles the child), and the
  line-cap policy evicts the oldest completed rows. UI keeps its frame
  budget; memory stays flat end-to-end.
- **Full-screen app takeover (`?1049`, vim/htop).** Alternate screen is a
  Phase 4 transposition. In Phase 1 such apps degrade: their redraws
  stream as linear styled text with `\r` rewrites; cards and UI are not
  suspended. Known limitation, documented by Design Decision 2026-11-07
  (dual engine).
- **PTY child death mid-command.** EOF finalizes the session (BR-9); any
  partially built final row is flushed; no zombies, no panic.
- **Command with no output.** The prompt itself streams as output; the
  document shows prompt rows and nothing else — no fabricated blank
  content (deferred auto-wrap rule prevents trailing empty rows, BR-5).
- **UTF-8 split across reads.** Multi-byte sequences spanning chunk
  boundaries reassemble (BR-5); no replacement characters injected by the
  chunking.
- **Grapheme edge cases (ZWJ/combining).** Explicit non-goal this phase:
  `unicode-width` scalar semantics apply, so composed emoji may measure
  wider/narrower than a full grapheme implementation. Documented
  degradation, Phase 4 grid inherits `unicode-width` too.
- **Cursor/DPR queries (`DSR`, `CPR`).** No responses are synthesized;
  programs that block on a cursor-position query may stall. Accepted
  degradation (Open Question 4).
- **Arrow keys under `APP_CURSOR`-mode applications.** Phase 1 always
  sends plain `CSI`; an application that enabled application-cursor mode
  may misread them. Accepted until the Phase 3 key-encoding layer
  (compat-first decision, roadmap Phase 3).
- **Backspace below column 0 / HT beyond width.** BS is clamped to
  floor 0; HT clamps at the PTY width; neither creates phantom content.
- **`ED`/`EL` while a colored style is active.** Erase implies painting
  with the active background at erase time — visual behavior pinned by
  golden test, not reinterpreted per app.

## Non-Functional Requirements

- **Memory:** standing budget < 80 MB RAM total (`knowledge/roadmap.md`
  Phase 0 floor ≈ 70 MB measured; this capability's delta stays within
  the 30–50 MB own-budget envelope). The session document, the channel,
  and the writer's internal state are all bounded by construction.
- **Frame budget:** the render thread keeps its frame budget under
  continuous streams; output ingestion is decoupled from rendering by the
  bounded channel (BR-2/BR-3).
- **Zero WebViews / zero Electron / zero Chromium** (architecture
  invariant): styling is native GPUI text rendering over spans.
- **Headless testability:** the writer and all persistence-free logic is
  unit-testable in headless mode (`cargo test`) without spawning GPU
  windows or PTY processes (conventions § Testing).

## Acceptance Criteria

- Given the engine starts, `/bin/zsh` runs through the PTY, its prompt
  streams visibly into the window, and typing a command executes it
  (passthrough: text, Enter, Backspace, Ctrl+C, Ctrl+D, Ctrl+L, arrows).
- Given 16-, 256-, and 24-bit SGR sequences plus bold/italic/underline
  attribute changes, rendered colors/attributes match the emitted
  attributes exactly for their duration.
- Given CJK/emoji output, each character occupies exactly its Unicode
  width (2 columns for wide glyphs) and auto-wrap accounts for it.
- Given `\r` + `\x1b[K` progress-bar rewrites, line content updates in
  place; given repeated writes at the last column, no empty rows are
  fabricated (deferred auto-wrap).
- Given a UTF-8 multi-byte sequence split across chunk reads, characters
  reassemble; golden fixtures confirm no mojibake.
- Given `EL` (0/1/2) and `ED` (0/1/2), the affected current-row region is
  erased with the active background; `ED 3` is ignored; unsupported CSI/
  OSC sequences never print.
- Given output exceeding the line cap, the document retains the newest
  content with drop-oldest eviction, memory stays bounded, and raw PTY
  bytes are all consumed (never dropped pre-writer).
- Given `cat /dev/urandom` and endless log floods, the window stays
  interactive at its frame budget and total RSS stays flat under
  sustained observation.
- Given a window resize, `PtyPort::resize(cols, rows)` fires immediately
  (no debounce), the shell receives `SIGWINCH`, new bytes wrap at the new
  width, and previously stored rows keep their original wrapping.
- Given the child dies mid-command (exit/crash/EOF), the session finalizes
  with the last content shown, no panic, no zombie.
- Golden tests (headless fixtures, raw byte inputs) cover all of:
  CJK/emoji widths, SGR palette, wrap at the last column, `\r` + `\x1b[K`
  rewrites, `EL`/`ED`, split UTF-8 reads, and the line-cap drop behavior.

## Access Control

Not applicable. Local single-user desktop application; the shell child
runs with the invoking user's privileges; the capability opens no network
sockets and performs no privileged operations beyond its own process's
PTY.

## Related Capabilities

- **osc-133-blocks** (Phase 2, planned): classifies this capability's byte
  stream into semantic command blocks; consumes the reader→channel→writer
  path and introduces per-block scope for the line cap (BR-4).
- **input-editor** (Phase 3, planned): replaces BR-8's keyboard passthrough
  with a native GPUI editor and the key-encoding layer.
- **grid-imperative-mode** (Phase 4, planned): swaps BR-5's subset for a
  full grid engine behind a Domain port; reuses BR-7's resize path.
- **session-history** (Phase 5+, planned): consumes PTY output for the
  Tools Panel cwd sync via the same stream.
- **performance-audit** (Phase 6, planned): virtualizes the bounded
  document rows produced here.

## Open Questions

1. **Restart semantics after shell death.** Phase 1 guidance: window stays
   open showing the last content; a new session requires relaunching the
   app. Whether an in-window "new session" action is product-promised needs
   a roadmap decision before implementation.
2. **Exact tunables.** Session-document line cap, channel capacity
   (bytes/items), HT tab-stop width (8 per VT convention), and the 16-color
   default theme palette are pinned in `design.md` — not user-visible
   behavior, not spec.
3. **Paste path.** Whether paste in Phase 1 goes through the same
   passthrough as typed keys (bracketed paste mode is unsupported this
   phase; multiline pastes split into lines with `CR` between them) or is
   deferred entirely to Phase 3 — implementation raises friction with
   shell-echo coupling; decision pending (see design tasks).
4. **`DSR`/`CPR` responses.** Confirmed non-goal for Phase 1 (no response
   synthesized). Revisit with the Phase 4 grid engine, where cursor
   position becomes answerable — applications that block on a query will
   wait until then.
