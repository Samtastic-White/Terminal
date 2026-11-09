# ⚡ Project Roadmap — Native Block Terminal (Rust + GPUI)

> **What this document is:** the project's single backlog. Every promised
> feature lives here as a user story with verifiable acceptance criteria, and
> this is the list a review uses to confirm features are delivered (or to
> catch ones that were forgotten).
>
> - `README.md` (and its Spanish translation `README.es.md`) answers "what
>   is the product?" (the features, in summary).
> - `knowledge/specs/<capability>/` answers "how is each capability's
>   implementation specified and gated?".
> - This document answers "what is left to do, in what order, and how is it
>   verified?".

## Feature Lifecycle

```text
1. Promised         → user story + acceptance criteria here.
2. In progress      → when its phase starts, invoke the SDD flow
                      (.agents/skills/create-specification/SKILL.md) and create
                      knowledge/specs/<capability>/ (proposal → spec → tasks).
                      No production code before the Tasks gate.
3. Implemented      → mark the item [x] linking its per-capability spec,
                      which remains living documentation (auditable with
                      .agents/skills/drift-audit against src/).
```

A phase closes only when **all** of its acceptance criteria are verified,
in addition to passing the standard verification (`cargo check`,
`cargo clippy -- -D warnings`, `cargo test` — `AGENTS.md` § 8).

---

## Phase 0 — Environment and Foundations

- [x] Cargo workspace with optimized profile (`lto = "thin"`, `opt-level = 3`).
- [x] `gpui` dependency linked via Git from Zed, added after passing the
      memory filter (STOP § 2). Pinned rev
      `9a8ab54db4f9f9af4dc6efb42b304c55c6a62bd3` (gpui 0.2.2,
      gpui_platform 0.1.0, wgpu 30).
- [x] Basic GPUI window opens on Wayland.
- [ ] Basic GPUI window opens on X11 — deferred by decision (2026-11-07):
      Xwayland's authorization cookie
      (`/run/user/1000/.mutter-Xwaylandauth.*`) is not mounted into the
      container. Revisit when needed (approved compose mount, host-side
      X11 run, or another authorization path; STOP § 5).
- [x] Rendering at 120 FPS — verified as sustained compositor pacing
      (Decision 2026-11-07): observed 143.8 Hz (gaps P50 6 952 µs) on the
      panel's current mode `1920x1080@144.002 Hz` (eDP-1, BOE); 43/4901
      single-frame drops (0.88%, compositor/VRR hiccups), zero multi-frame
      starvation. Note: the strict P99 (14 050 µs, above the
      observed-period tolerance of 8 690 µs) stays documented as a known
      note, not a blocker.
- [x] Idle RAM, reformulated from measurement (Decision 2026-11-07): the
      original `idle < 30 MB` weighed only the luggage, while the probe
      weighs the whole building. Measured idle PSS ≈ 70 MB, attributed:
      ~35 MB GPU driver stack (libLLVM 24.6 MB, libvulkan_intel 4.6 MB,
      libvulkan_lvp 2.7 MB, libvulkan_dzn 1.3 MB), ~27 MB gpui engine
      (binary text 16.4 MB + initial heap 10.3 MB), ~8 MB shared system
      libs. The 30–50 MB budget governs the app's own delta over that
      floor; the absolute ceiling < 80 MB is the verified criterion in
      Phase 6 (70 MB observed today).

> **Phase 0 verification note (2026-11-07).** Measurements come from the
> permanent env-gated probe (`NT_MEASURE=1`; repro:
> `docker compose exec -e NT_MEASURE=1 dev cargo run --release`; window
> focused ~60 s: 15 s settle + 10 × 2 s samples). Metric: PSS via
> `/proc/self/smaps_rollup`. Monitor modes read from mutter
> (`org.gnome.Mutter.DisplayConfig.GetCurrentState`): `eDP-1` (BOE),
> `1920x1080@144.002 Hz` is-current + is-preferred (VRR-capable).
> Pitfall learned: gpui_linux's Wayland `FrameLoop` retry timer
> (`FRAME_RETRY_INTERVAL = 16_667 µs` = 60 Hz) double-paces a client-side
> `on_next_frame` chain; pacing must be driven and measured through the
> compositor-paced `Animation` path (`max_fps: None`). Probe verdicts
> anchor to the observed p50 period. Standard verification passed:
> `cargo check`, `cargo clippy -- -D warnings` (dev + release),
> `cargo fmt --check`, `cargo test`. Phase 0 closes with the X11 checkbox
> explicitly deferred above.

## Phase 1 — PTY Subsystem and Raw Output

> **Decision (2026-11-07):** the engine is dual. Normal-mode output is
> interpreted by a bounded `vte::Perform` implementation ("block text
> writer") in `src/infrastructure/parser/`. A full terminal-grid state
> machine is NEVER hand-rolled; a proven crate (`alacritty_terminal` or
> `vt100`) is adopted in Phase 4 behind a Domain port. `vte` is only the
> Paul Williams escape FSM (zero state: no cursor, grid, scrollback, or
> Unicode widths). The per-capability spec under `knowledge/specs/`
> (SDD flow) defines the writer's exact subset when this phase begins.

- [ ] `portable-pty` spawns `/bin/zsh` (crates approved under STOP § 2:
      `portable-pty`, `vte`, `unicode-width`).
- [ ] PTY byte-stream reading runs on a background worker and reaches the UI
      through a channel; the render thread is never blocked
      (`AGENTS.md` § 9).
- [ ] `BlockTextWriter` (bounded `vte::Perform` subset): plain text,
      CR/LF/BS/HT, 16-/256-/24-bit SGR colors, style attributes
      (bold/italic/underline), `EL`/`ED`, auto-wrap at PTY width, and
      cursor tracking sufficient for `\r` rewrites (progress bars).
- [ ] Non-goals (deferred to the Phase 4 grid engine): scroll regions,
      origin mode, alternate screen (`?1049`), charsets, full cursor
      save/restore semantics, scrollback reflow.
- [ ] Unicode widths via `unicode-width` (CJK double-width; grapheme
      cluster edge cases are an explicit non-goal this phase).
- [ ] No output accumulates in an unbounded `String`/`Vec`: the writer is
      bounded by PTY width per line plus a per-block line cap with a
      drop policy (`knowledge/architecture.md` Common Mistakes #4/#5).
- [ ] Golden tests (headless, hand-maintained fixtures): CJK/emoji
      widths, SGR palette, wrap at the last column, `\r` + `\x1b[K`
      rewrites, `EL`/`ED`, UTF-8 multi-byte sequences split across chunk
      reads, and the line-cap drop behavior.
- [ ] GPUI window resize is forwarded immediately (no debounce) to
      `PtyPort::resize(cols, rows)` (the kernel sends `SIGWINCH` to the
      foreground group); the block text writer wraps newly arriving
      bytes at the new PTY width. In Phase 4 the same path resizes the
      grid engine.
- [ ] The PTY→UI channel is bounded with real backpressure: when full,
      the reader parks and the child process is throttled by the kernel
      pipe buffer. PTY bytes are never dropped (post-emulator line caps
      only); the render thread keeps its frame budget under continuous
      streams (`cat /dev/urandom`, endless logs).

## Phase 2 — OSC 133 Semantic Protocol and First Visual Block

> **Decision (2026-11-07):** the state machine is tolerant by design — the
> linear happy path (A → B → C → D) is NOT assumed. Cold start (the first
> event ever is `D;0`, before any `A`), aborted lines (Ctrl+C or empty
> Enter produce `D` without `B`/`C`), nested integrated shells, and shells
> that die without `D` (`exit`, crash, PTY EOF) are first-class cases. OSC
> payloads are payload-strict (Decision 2026-11-07): only the exact
> `Cwd=` prefix opens the Cwd event — the semantic-prompts standard
> reserves bare `133;P` for secondary/continuation prompts, so those are
> silently ignored; incomplete or unknown payloads are dropped, never
> panicked on. The pure FSM lives in `src/domain/` (headless-testable);
> byte-level OSC detection stays in `src/infrastructure/parser/`.

- [ ] Injected Zsh hooks emit the standard markers, engine-gated
      (`NATIVE_TERMINAL=1` in the PTY child env) and nesting-guarded
      (`NATIVE_TERMINAL_ACTIVE` exported by the parent suppresses hooks
      in nested interactive shells).
- [ ] Rust state machine classifies A / B / C / D / Cwd / `633;E` per the
      tolerant transition table (§ Block State Machine): `D` without
      prior `C` is ignored, `A` while running is ignored, duplicate and
      unknown markers are ignored (never printed), `633;P;Cwd` accepted
      in every state.
- [ ] Cold start: bytes before the first `A` (banner, `.zshrc` noise) are
      swallowed; the first `D;0` creates no phantom block.
- [ ] Silence windows use bounded ring buffers (never unbounded): before
      the first `A`; between `A` and `B` (PS1 text, user echo, ZLE
      redraws); between `B` and `C`; between `D` and `A` (except
      `633;P`).
- [ ] Block header receives the exact executed command via `OSC 633;E`
      (`preexec` `$1`); ZLE echo reconstruction is forbidden (fragile).
- [ ] Block working-directory snapshot is taken at `C` from the last
      `633;P;Cwd` (a Cwd arriving after `D` belongs to the next block).
- [ ] Shell death without `D` (user `exit`, crash, PTY EOF, terminal
      reset `\e`): the running block finalizes as `Interrupted`
      (grey badge, exit code "—", never red).
- [ ] First visual `CommandBlock`: independent card with rounded border,
      margins, and per-block state.
- [ ] Block header: executed command (via `633;E`), relative time
      ("2 min ago"), duration ("1.42s"), and exit-code badge (`0` green,
      `>0` red, `Interrupted` grey).
- [ ] Output body: full ANSI highlighting, free mouse selection, local scroll.
- [ ] Functional copy button (first per-block action).
- [ ] Every byte between `C` and `D` is routed exclusively to the active
      block's buffer.
- [ ] Continuous streaming lands inside the active block under the
      per-block line cap: UI stays responsive and memory stays flat
      end-to-end (reader → bounded channel → capped block model).

## Phase 3 — GUI Input Editor and Sticky Header

- [ ] Native GPUI input component replaces ZLE: multiline editor.
- [ ] Cursor positionable by click, drag selection, and `Shift + Arrows`.
- [ ] Native shortcuts: `Ctrl + C` (copy selection), `Ctrl + V` (paste without
      breaking indentation), `Ctrl + A` (select the whole command).
- [ ] `Shift + Enter` for a clean line break; `Enter` locks the editor and
      sends the packaged string to the PTY for immediate execution.
- [ ] Engine mode silences the PTY's own `A…B` region (empty `PS1`,
      `stty -echo`, ZLE disabled via the integration script): the GPUI
      editor is the sole source of command text — no ghost prompt or
      echoed bytes leak between blocks.
- [ ] Sticky Command Header: when output exceeds the visible height, the block
      header pins to the top edge of the viewport.
- [ ] When scrolling past the end of the block, the pinned header yields with a
      smooth offset to the next block.
- [ ] Key-encoding layer (compat-first, Decision 2026-11-07): keys the
      editor sends to the PTY are encoded in the dialect the foreground
      application expects — arrows use `SS3` under `APP_CURSOR` mode and
      raw `CSI` otherwise; modified keys encode as `CSI 1;<mod>`. The
      kitty keyboard protocol (`CSI u`) stays in the Backlog as
      progressive enhancement that degrades gracefully (apps query and
      fall back to legacy encodings when unsupported).

## Phase 4 — Immersive Mode (Alternate Screen)

- [ ] Grid engine adopted from a proven crate (`alacritty_terminal` or
      `vt100`) behind a Domain port — chosen and memory-filtered in this
      phase's SDD spec (Decision 2026-11-07: no hand-rolled full grid).
- [ ] The parser detects `\x1b[?1049h` and `\x1b[?1049l`.
- [ ] Live size sync: the alt-screen grid viewport always matches the
      PTY size (the Phase 1 resize path feeds the grid engine).
- [ ] Mouse forwarding: when the application enables mouse reporting
      (DECSET `?9`/`?1000`/`?1002`/`?1003`, encoding X10 or SGR
      `?1006`), GPUI scroll/click/drag events are encoded and written
      to the PTY instead of local scrolling/selection; with `?1007`
      enabled, the wheel maps to Up/Down keys.
- [ ] With interactive tools (`vim`, `nvim`, `htop`, `nano`, `less`), cards and
      the input bar are suspended; the screen buffer takes 100% of the GPUI
      viewport.
- [ ] On exit, the block view is restored exactly where it was.

## Phase 5 — Tools Panel (Sidebar File Explorer)

- [ ] Two-column layout with resizable splitter; collapsible panel with a
      shortcut (reference: `Ctrl + B`).
- [ ] Cwd file tree synchronized by the
      `\e]633;P;Cwd=...\e\` event: after `cd`, refresh in milliseconds with no
      manual intervention.
- [ ] Filesystem events (`notify`) refresh the tree live and off the render
      thread.
- [ ] Icons by file type, hidden-file visibility, and basic Git status.
- [ ] Clicking a text/code file: preview or open in the default editor
      (e.g. Zed). Executable/directory: navigate or execute.

## Phase 6 — Performance and Memory Audit

> **Decision (2026-11-07):** outer block virtualization uses `gpui::list`
> + `ListState` — the dynamic-height list already shipped in GPUI is
> consumed, not rebuilt. Per-block rendering for giant visible blocks
> uses a custom element with content-mask culling (the same pattern as
> Zed's `TerminalElement` and our Phase 4 grid element).

- [ ] Profiling with `heaptrack` and `ps_mem` executed and documented.
- [ ] Block virtualization: outer `gpui::list` + `ListState` with dynamic
      heights (measure only visible + overdraw; `FollowMode::Tail` for
      streaming; `remeasure_items` for the growing/collapsing active
      block; O(1) monospace height hints `line_count × row_height`),
      plus per-block content-mask line painting; off-screen blocks are
      never laid out. `uniform_list` and plain `div` + `overflow_scroll`
      are rejected for the main block list. Data outside the viewport is
      still bounded by the per-block line cap (freed or compressed,
      never kept unbounded).
- [ ] Proven ceiling: the app's own delta stays within 30–50 MB on the
      Phase 0 measured platform floor (idle PSS ≈ 70 MB: ~35 MB GPU driver
      stack + ~27 MB gpui engine); absolute total ceiling < 80 MB RAM.

---

## Backlog — Promised Features (No Phase Assigned)

Stories recorded as verifiable promises; `README.md` § 3 summarizes them.
When implementation is scheduled, a spec is written for them using the SDD
flow.

- [ ] **Per-block selective copy**: on hover, 4 actions — copy only the
      command, copy only the output (stdout/stderr without prompt), copy the
      whole block, collapse/expand the card.
- [ ] **Rich, filterable history** (Phase 5+, `rusqlite`): visual panel with
      previous commands, date/time, duration in seconds, exit code
      (success/failure), and per-project filters.
- [ ] **Smart autocomplete (IDE-style)**: while typing (e.g. `git`, `docker`),
      a menu opens with flags explained visually (taken from `<cmd> --help`).
- [ ] **Terminal splits**: split the window into horizontal and vertical
      panes.
- [ ] **Vertical tabs**: sidebar tab bar that does not take height away from
      the block viewport.
- [ ] **Kitty graphics passthrough** (image history): inline image
      rendering in blocks; protocol target kitty graphics, sixel as
      follow-up if demanded. Efficiency is part of the story, not an
      afterthought: decode on a worker thread (never the render thread),
      raw bytes discarded right after texture upload, decoded-image
      cache capped at ~6 images with viewport-driven LRU eviction,
      downscale to display size before decoding when the format allows,
      and zero-leak verification with `ps_mem` during the Phase 6 audit.

---

## OSC 133 Semantic Protocol (Cross-Cutting Standard)

Marker table and engine effects: `AGENTS.md` § 10 (single source).
Phases 2, 3, and 5 consume this standard; changing it requires updating that
table and the affected per-capability specs.

### Canonical Zsh Script

Engine-gated and nesting-guarded, sourced once per interactive shell.
The engine puts `NATIVE_TERMINAL=1` in the PTY child environment at
spawn. `print -Pn` is safe for payload-free markers, but cwd values may
contain `%` or `;`, so `Cwd` uses `print -rn` (literal) and the parser
takes everything after the `Cwd=` prefix as the path (take-rest, no
splitting on `;`). `633;E` carries the exact command line from `preexec`
`$1` (VS Code convention); its payload is take-rest as well. Newlines
inside `$1` are lost inside OSC (the parser drops control bytes in OSC
state) — acceptable degradation for single-line headers.

```zsh
# OSC 133 semantic integration for the terminal (engine-gated, nesting-guarded)
if [[ $- == *i* && -n ${NATIVE_TERMINAL:-} ]]; then
  if [[ -z ${NATIVE_TERMINAL_ACTIVE:-} ]]; then
    export NATIVE_TERMINAL_ACTIVE=1   # nested zsh inherits this and stays silent

    precmd() {
      print -Pn  "\e]133;D;$?\e\\"    # D: previous command finished (with exit code)
      print -rn "\e]633;P;Cwd=$PWD\e\\"  # Cwd: literal cwd (print -P would eat % in paths)
      print -Pn  "\e]133;A\e\\"       # A: prompt starts
    }

    preexec() {
      print -Pn "\e]133;B\e\\"        # B: prompt ends, input begins
      print -Pn "\e]133;C\e\\"        # C: command executed, output begins
      print -Pn "\e]633;E;$1\e\\"     # E: exact command line (VS Code convention)
    }
  fi
fi
```

### Block State Machine (Rust)

Three states plus `Closed`; tolerant by design — the happy path is a
subset, never a requirement. The exact transition table (timers,
ordering guarantees, chunk boundaries) is pinned by the Phase 2 SDD
spec; the load-bearing rules:

| State | Event | Next state | Action |
| --- | --- | --- | --- |
| `Cold` | `A` | `Prompting` | first prompt reached |
| `Cold` | `D;<n>` | `Cold` | ignore — a born shell emits `D;0` before any `A` |
| `Prompting` | `B` | `Prompting` | hint only: expect `C` next |
| `Prompting` | `C` | `Running` | open block; snapshot cwd from last `P`; command text from `633;E` |
| `Prompting` | `D;<n>` | `Prompting` | ignore — empty Enter / Ctrl+C at the prompt (no `B`, no `C`) |
| `Running` | `D;<n>` | `Prompting` | complete block: exit code + elapsed time recorded |
| `Running` | `A` | `Running` | ignore — nested-shell tolerance |
| `Running` | `C` | `Running` | close current block (as if interrupted) and open a new one (flat list) |
| PTY EOF / `\e` / shell death (any state) | — | `Closed` | a running block finalizes as `Interrupted` |

`633;P;Cwd` is accepted in every state (notification, never a
transition). Payload-strict: bare `133;P` (continuation prompt in the
semantic-prompts standard) or non-`Cwd` variants are unknown noise,
silently ignored — never a directory event. Duplicate and unknown
markers are ignored (never printed). Every byte outside `C…D` routes to
its silence window, never to a block buffer.
The `Running`+`C` row is exotic (the nesting guard should prevent it)
and degrades deterministically by splitting the block list.
