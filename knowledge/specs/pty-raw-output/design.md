# Design — PTY Subsystem and Raw Output

> Phase 1. Derives from `spec.md` (Gate 2 approved). Layers, ports and
> lifecycles use `knowledge/architecture.md` vocabulary. This document pins
> cross-layer contracts, threading, the document model, and the tunables
> the spec defers here.

## 1. Layer Decomposition (Change Mapping)

| Piece | Layer / path | Rationale |
| --- | --- | --- |
| `PtyPort` trait, `PtySize` | `src/domain/pty.rs` | Port contract; Domain imports nothing OS-specific |
| Output primitives (`Row`, `Span`, `TextStyle`) | `src/domain/output.rs` | Pure data the Application model stores; gpui converts to rendering only in Presentation (Common Mistake #7: no ANSI/parsing in views) |
| `PtyAdapter` (portable-pty), reader worker, bounded channel producer | `src/infrastructure/pty/mod.rs` | Infrastructure implements the Domain port |
| `BlockTextWriter` (`vte::Perform`) | `src/infrastructure/parser/mod.rs` | Byte-level interpretation; the only place escape literals may appear (conventions § Shell Protocol Hygiene) |
| `SessionService` + `SessionDocument` | `src/application/session.rs` | Orchestration: spawn/write/resize/ingest; gpui-free |
| `TerminalError` variants | `src/common/errors.rs` | New `Pty`, `Parse`, `Channel` variants (conventions § Error Handling) |
| `SessionView` (render surface + key capture + resize hook), `theme` | `src/presentation/session_view.rs`, `src/presentation/theme.rs` | Driving adapter; translates GPUI events into Application calls |
| Composition root wiring | `src/main.rs` | Builds adapters, injects into services; **Phase 0 probe untouched** |

Dependency direction stays one-way inward; Infrastructure never touches
Presentation or gpui types.

## 2. Contracts

### `PtyPort` (Domain; headless-testable)

```rust
pub trait PtyPort {
    /// Spawn the child shell. Idempotent per session: a second call is a
    /// no-op while a live session exists (BR-1).
    fn spawn(&mut self, size: PtySize) -> Result<(), TerminalError>;
    /// Write raw bytes (keystroke passthrough, BR-8). Non-blocking write;
    /// short writes are completed by the adapter before returning Ok.
    fn write(&mut self, bytes: &[u8]) -> Result<(), TerminalError>;
    /// Resize cols/rows; the kernel delivers SIGWINCH (BR-7).
    fn resize(&mut self, size: PtySize) -> Result<(), TerminalError>;
    /// True once EOF/death was observed (BR-9 finalization source).
    fn is_alive(&self) -> bool;
}
```

Child environment pinned at spawn: `NATIVE_TERMINAL=1` (Phase 2 hooks read
it later, harmless now), `TERM=xterm-256color`, `COLORTERM=truecolor`
(required so zsh/tools actually emit the SGR classes the writer supports).
Command: `/bin/zsh` interactive login shell.

### Channel contract (the backpressure seam)

- One dedicated **OS thread per session** performs blocking `read()` into a
  fixed `PTY_BUFFER_KIB` buffer — blocking there is correct; it is not the
  render thread (BR-2).
- Chunks travel through a bounded `std::sync::mpsc::SyncSender` with
  capacity `CHANNEL_CAP` chunks. `send()` blocking when full = the reader
  parks; the kernel pipe buffer (64 KiB default) then throttles the child
  (BR-3). No PTY bytes are ever dropped in that path.
- Consumer side wakes through the GPUI **foreground executor** as an async
  task (`cx.spawn`): drain up to N chunks per wake with `try_recv`; when
  the channel is momentarily empty, await `executor.timer(POLL_INTERVAL)`.
  The drain applies chunks through `SessionService::ingest_bytes` and then
  triggers `cx.notify()`. The render thread is never blocked and the drain
  task is never on the render loop; an 8 ms max latency is within frame
  budget. If `gpui_platform` ships a wakeable blocking channel primitive at
  implementation time, prefer it and drop the timer — the pinned behavior
  is: bounded, non-dropping, non-render-blocking wake-up.

### Writer/data model

- `BlockTextWriter` wraps `vte::Parser` + implements `vte::Perform` for the
  BR-5 subset only. State: current `TextStyle` (fg/bg/attrs), cursor col in
  the current logical row, pending-wrap flag, UTF-8 reassembly handoff
  (vte feeds the decoder; the writer receives finished `char`s).
- Emission: completed logical rows are pushed as `Row`s; `\r` rewrites
  mutate the **current** row in place (segment-level overwrite from the
  cursor col; `EL/ED` truncates/paints with the active background). Rows
  are immutable once committed except by this in-place rewrite machinery.
- Auto-wrap: at last column the writer sets pending-wrap; the next printed
  character commits the row and opens the next one (spec BR-5). `CR`/`BS`/
  `HT` clear pending-wrap (any explicit cursor motion does).
- Column advancement uses `unicode_width::UnicodeWidthChar`; `HT` snaps to
  `TAB_STOP` multiples; every row's total columns ≤ PTY cols at wrap time
  (hard bound; no fragmentation below that).
- The writer knows cols only as a number fed at construction/resize — it
  stays window-agnostic and fully headless-testable.

### `SessionDocument` + `SessionService` (Application)

- `SessionDocument`: `VecDeque<Row>` capped at `OUTPUT_MAX_ROWS`; cold path
  on eviction (`pop_front`), newest always retained (BR-4). Tracks its
  wrap-width snapshot per row (rows keep creation-time wrapping, BR-7).
  Eviction discards interpreted rows only — the raw byte path upstream is
  untouched.
- `SessionService` (gpui-free): `spawn_session`, `ingest_bytes(&[u8])` →
  writer → document, `write_keystrokes`, `resize(PtySize)`, `is_alive`,
  read access to the document Presentation renders. Presentation calls it
  and converts results to frames — never `div()` here (Application rule).

## 3. Execution flow (inbound, Phase 1 shape)

```text
zsh ──bytes──► PTY reader OS thread (blocking read, fixed buffer)
      │ bounded SyncSender (parking send = kernel throttles child)
      ▼
GPUI foreground drain task (cx.spawn; try_recv → timer, non-render thread)
      ▼
SessionService::ingest_bytes ──► BlockTextWriter (vte Perform subset)
      ▼
SessionDocument (VecDeque<Row>, drop-oldest cap)
      ▼
Presentation SessionView render (styled text) + cx.notify()
```

Outbound (BR-8): GPUI key event → Presentation maps keycode/char to bytes
(pinned table below) → `SessionService::write_keystrokes` →
`PtyPort::write`. Presentation never holds the PTY handle.

## 4. Tunables & pinned defaults

| Constant | Value | Notes |
| --- | --- | --- |
| `OUTPUT_MAX_ROWS` | 4 000 | session-document cap, drop-oldest (BR-4) |
| `CHANNEL_CAP` | 64 chunks | ≈ 512 KiB in-flight maximum |
| `PTY_BUFFER_KIB` | 8 KiB | reader read size |
| `TAB_STOP` | 8 | VT convention |
| `POLL_INTERVAL` | 8 ms | drain-task idle wake; replaced if a wakeable channel exists |
| Theme palette | Catppuccin-like | the 0x181825/0xcdd6f4 family already rendered in Phase 0; full 16-color table coded in `src/presentation/theme.rs` |
| Font | default monospace | row height from font metrics yields cols/rows for `PtySize` |

Exact values are implementation-tunable; the *policy* is spec-level and
does not change.

## 5. Keyboard passthrough table (BR-8)

| GPUI key | Bytes |
| --- | --- |
| printable char (ASCII/UTF-8) | its UTF-8 encoding |
| Enter | `0x0D` (CR) |
| Backspace | `0x7F` (DEL) |
| `Ctrl+C` / `Ctrl+D` / `Ctrl+L` | `0x03` / `0x04` / `0x0C` |
| Arrows ↑↓←→ | `CSI A/B/C/D` (`0x1B 0x5B …`) plain (APP_CURSOR mismatch accepted until Phase 3) |
| anything else | dropped silently (never wrongly encoded) |

Bracketed paste: unsupported this phase; pasted text feeds the same path
line by line with `CR` separators (spec Open Question 3 resolved on the
conservative side; revisit with Phase 3 editor).

## 6. Resize path

GPUI window-resize event → Presentation recomputes cols/rows from content
size ÷ row metrics → `SessionService::resize(PtySize)` → `PtyPort::resize`
(Immediate, no debounce — kernel SIGWINCH) + document keeps stored rows
(BR-7: no retroactive re-wrap) + writer updates its wrap width for the
next chunk. Phase 4's grid engine plugs into this same path.

## 7. Golden test harness (headless)

- Fixtures are **Rust constants** (`&[(&[u8], Vec<Row>)]`) colocated in the
  parser module under `#[cfg(test)]` — no runtime file I/O, no fixture
  crates (conventions § Testing; minimal-dependency bias).
- The seven spec families are covered as named test functions: widths
  (CJK/emoji), SGR palette, wrap at last column, `\r` + `\x1b[K` rewrite,
  EL/ED (incl. active-background painting), split UTF-8 across chunks,
  line-cap eviction order (drop-oldest, newest retained).
- Writer tests construct the writer with explicit cols — no PTY, no GPU,
  no window (`cargo test` fully headless).
- Bounded-channel/backpressure: a unit test parks a full `SyncSender` and
  asserts `send` blocks and wakes on drain (no PTY involved).

## 8. Errors & failure mapping

- PTY open/spawn failures, `write` errors → `TerminalError::Pty`.
- Writer/parser can't error (BR-6: degrade deterministically); channel
  send failure on a closed receiver after EOF → reader thread exits
  quietly (session ended — BR-9), mapped at the infrastructure boundary
  only (conventions § Error Handling).
- No `unwrap`/`expect` in production paths; reader thread panics are
  impossible by construction (all fallible ops matched).
- EOF: reader loop ends → consumer task drains final chunks → writer
  flushes the partially built row → document finalized; UI keeps showing
  content. No auto-restart (spec Open Question 1 is left to a roadmap
  decision; implementation does not invent it).

## 9. Dependencies (STOP § 2 scope)

`portable-pty`, `vte`, `unicode-width` enter `Cargo.toml` — approved in the
roadmap Phase 1 bullet. Nothing else (probe stays std-only; document
`VecDeque` is std). Provenance guard stays empty
(`rg -i "denver technologies|license-alacritty|warp_terminal|waveterm" src/`).

## 10. Explicit omissions (= spec non-goals implemented by absence)

Scroll regions, origin mode, alternate screen (`?1049`), charsets, full
cursor save/restore, scrollback reflow, DSR/CPR responses, mode-dependent
key encodings (SS3/kitty). Each omission lives as "ignored" branches in the
writer with a one-line comment pointing at the Phase 4 handoff, so the
drift-audit can trace intent.
