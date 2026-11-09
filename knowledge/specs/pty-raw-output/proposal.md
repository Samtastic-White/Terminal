# Proposal — PTY Subsystem and Raw Output (Phase 1)

> Status: DRAFT — awaiting Gate 1 approval.
> Phase source: `knowledge/roadmap.md` § Phase 1 (Decision 2026-11-07: dual
> engine — `vte` interprets normal-mode output now; the full grid engine is
> adopted in Phase 4 behind a Domain port).

## Problem / Motivation

The engine opens a GPUI window on Wayland but has no shell: nothing spawns
`/bin/zsh`, no command can run, and raw output is never displayed. Phase 1
connects the PTY lifecycle and interprets raw child output with bounded
memory — the foundation every later phase (semantic blocks, input editor,
grid imperative mode) builds on.

## Scope

New capability, two halves:

1. **PTY subsystem** — spawn, own, read, write to, and resize `/bin/zsh`
   through `portable-pty`. Reading happens on a background worker; bytes
   reach the UI through a bounded channel with real backpressure (kernel
   pipe buffer throttles the child; bytes are never dropped).
2. **Raw output interpretation** — `BlockTextWriter`, a bounded
   `vte::Perform` implementation in `src/infrastructure/parser/`: plain
   text, CR/LF/BS/HT, 16-/256-/24-bit SGR colors, style attributes
   (bold/italic/underline), `EL`/`ED`, auto-wrap at PTY width, minimal
   cursor tracking for `\r` rewrites (progress bars), CJK double-width via
   `unicode-width`.

Explicit non-goals (deferred to the Phase 4 grid engine): scroll regions,
origin mode, alternate screen (`?1049`), charsets, full cursor
save/restore, scrollback reflow. OSC 133 semantics are Phase 2 — this
phase renders the noise, it does not classify it.

## Impact

- Who/what is affected: the engine gains its first real terminal behavior;
  UI output rendering appears in Presentation; `PtyPort` becomes the first
  Domain port consumed by Application; parser layer is born in
  Infrastructure.
- Dependencies added (pre-approved in the roadmap under STOP § 2):
  `portable-pty`, `vte`, `unicode-width`. The flow uses them at
  implementation time; no other crates may enter.
- Later phases affected: Phase 2 (blocks) consumes the PTY→UI byte path
  and the writer's style model; Phase 4 (grid) reuses the resize path;
  Phase 6 (virtualization) builds on the bounded per-block line cap
  policy.

## Draft User Stories

- As a user, I want to run commands in zsh and see their raw output in the
  window, so that the terminal is finally usable.
- As a user, I want colors and styles from my shell and tools to render
  correctly, so that output stays readable.
- As a user, I want the window to stay responsive under endless output,
  so that streaming logs never freeze the UI or grow memory unboundedly.
- As a user, I want window resize to reach running programs immediately,
  so that newly wrapped output matches the pane width.

## Draft High-Level Acceptance Criteria

- Given the engine starts, `/bin/zsh` runs through the PTY and its prompt
  streams visibly into the window.
- Given 16-, 256-, and 24-bit SGR sequences plus bold/italic/underline,
  colors and attributes render as emitted.
- Given CJK/emoji text, each character occupies its Unicode width.
- Given `\r`-based rewrites (`\r` + `\x1b[K`), lines rewrite in place and
  progress bars behave.
- Given UTF-8 multi-byte sequences split across chunk reads, characters
  reassemble correctly.
- Given output exceeding the per-block line cap, memory stays bounded by
  the documented drop policy — while PTY bytes themselves are never
  dropped before the writer.
- Given a window resize, `PtyPort::resize(cols, rows)` is forwarded
  immediately (no debounce; kernel sends `SIGWINCH`) and new bytes wrap at
  the new width.
- Given a continuous flood (`cat /dev/urandom`, endless logs), the render
  thread keeps its frame budget and the bounded channel parks the reader
  instead of accumulating.
- Golden tests (headless fixtures) cover: CJK/emoji widths, SGR palette,
  wrap at the last column, `\r` + `\x1b[K` rewrites, `EL`/`ED`, split
  UTF-8 multi-byte reads, and line-cap drop behavior.

## Gates

Gate 1 (this proposal) → `spec.md` → Gate 2 → `design.md` (expected:
yes — new cross-layer contracts: `PtyPort`, reader worker lifecycle,
writer/document model) → `tasks.md` → Gate 3 → implementation.
**No production code until Gate 3 is approved.**
