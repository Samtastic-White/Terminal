> 🇬🇧 **English** · [🇪🇸 Español](./README.es.md)

# ⚡ Native Block Terminal (Rust + GPUI)

> **Canonical English source of truth.** Spanish translation: [`README.es.md`](./README.es.md). If they disagree, this file wins.

> **Native re-engineering and migration of Wave Terminal's architecture to Rust and GPUI, with Warp's visual experience and ergonomics.**
> A high-performance block-based terminal emulator for Linux, rendered directly on the GPU (Vulkan/Wayland) and designed under a strict **under-80 MB RAM** budget, with no Electron, Node.js, or web browser engines (zero WebKitGTK, zero Chromium).

---

## 1. The Project Thesis

The world of Linux terminal emulators is broken into two polarized extremes:

| Terminal | Visual Model | Input Editor | RAM Usage | Architecture |
| :--- | :--- | :--- | :--- | :--- |
| **Ghostty / Kitty / Alacritty** | Continuous grid (VT100) | Plain ZLE (ANSI) | 35 – 60 MB | Native (Zig / C) |
| **Warp Terminal** | Cards / Blocks | Rich GUI editor | 550 – 800 MB | Rust + Cloud / Telemetry |
| **Wave Terminal** | Blocks / Canvas | Monaco (VS Code) | 600 – 1000 MB | Electron (Chromium) + Go |
| **This project** | **Cards / Blocks** | **GUI Editor (GPUI)** | **50 – 80 MB** | **Pure Rust + Native GPUI** |

- **The classic continuous-grid model** is unbeatable in performance, but it carries the conceptual limitations of the 40-year-old teletype: text as a flat, unstructured buffer, a second-class mouse, and an input editor (ZLE) where multiline selection or click-to-move-cursor is crude or impossible.
- **Modern block terminals** turn the console into a real interface — each command is an independent card with state, metadata, sticky headers, and a multiline graphical editor — but at a brutal price: Wave loads **Electron + Chromium + React 19 + Monaco + a Go daemon** (600 MB – 1 GB), and Warp couples a Rust client to proprietary cloud infrastructure and telemetry (500 – 800 MB).

**Value proposition:** Wave's block architecture and workspace, Warp's visual ergonomics, and the resource ethics, speed (120 FPS), and < 80 MB footprint of Ghostty or Zed — all in native, GPU-accelerated Rust.

---

## 2. The Central Strategy: Architectural Migration from Wave to GPUI

This project is not a terminal built at random: **it is a direct conceptual port of Wave Terminal's architecture**, replacing its web execution engine with native systems technologies. Wave brilliantly solved how to model a terminal block, its controller, and its lifecycle; the work here is to take that proven architecture and rebuild it in Rust.

```text
┌──────────────────────────────────────────────────────────────────────────┐
│                        WAVE'S ORIGINAL ARCHITECTURE                      │
│   Frontend: Electron + Chromium + React 19 + Monaco (~700 MB RAM)        │
│   Backend:  Local Go daemon (wavesrv) + WebSockets + PTY                 │
└────────────────────────────────────┬─────────────────────────────────────┘
                                     ▼  (ARCHITECTURAL MIGRATION)
┌──────────────────────────────────────────────────────────────────────────┐
│                        OUR NATIVE ARCHITECTURE                           │
│   Frontend: GPUI (Zed Engine) ── direct Vulkan rendering (~70 MB)        │
│   Backend:  In-memory Rust modules (portable-pty) ── mpsc channels       │
└──────────────────────────────────────────────────────────────────────────┘
```

### Migration Equivalence Table

| Component | Original Wave Implementation | Rust / GPUI Reimplementation |
| :--- | :--- | :--- |
| Shell / Renderer | Electron + Chromium | Native GPUI (hardware-accelerated, no browser) |
| Command Editor | Monaco Editor (VS Code web) | Native GPUI Input Editor (text spans) |
| Block Model | `BlockController` (TS / Jotai) | Reactive `Model<CommandBlock>` (`cx.notify()`) |
| Terminal Backend | Go server (`wavesrv`) over sockets | Internal PTY adapter with `portable-pty` (direct RAM) |
| ANSI / Control Parser | `xterm.js` | Stateful lexical parser + semantic sequences |
| Workspace / Splits | React layout tree | GPU-accelerated Flexbox layouts in GPUI |

---

## 3. The Experience: Warp's Elegance

The internal engineering comes from the Wave migration; the interface, aesthetics, and interaction replicate Warp's elegance:

- **Physical isolation per card** — the screen is not a continuous stream of text: each command creates a visual card (`CommandBlock`) with its own container, rounded borders, and margins.
- **The “No Infinite Scroll” (Sticky Command Header)** — in outputs of thousands of lines (logs, builds), the block header freezes at the top edge so context is never lost.
- **Selective one-click copy** — per-block buttons to copy only the command, only the output (stdout/stderr, without prompts), or the whole block.
- **GUI Input vs. ZLE** — the input bar is a real editor: click-to-position cursor, drag selection, system shortcuts (`Ctrl+C/V/A`), and clean line breaks with `Shift+Enter`.
- **Immersive Alt-Screen mode** — detection of interactive tools (`vim`, `htop`, `less`): the system suspends cards and gives 100% of the viewport to the process to avoid broken redraws.
- **Tools Panel** — sidebar with a file tree of the current cwd updated in real time (via the shell's semantic event), file preview, and file opening without typing `ls`.

---

## 4. Semantic Shell Integration (Zsh + OSC 133)

So the engine knows when a command starts, when it ends, and what code it produced — without heuristics or fragile regular expressions — standard OSC 133 semantic hooks are injected into `/bin/zsh`:

| Marker | Meaning |
| :--- | :--- |
| `\e]133;A\e\` | Prompt start: the shell waits for commands |
| `\e]133;B\e\` | Prompt end: the user starts typing |
| `\e]133;C\e\` | Command executed: process starts, active block is instantiated |
| `\e]133;D;<code>\e\` | Command finished: exit code and duration captured |
| `\e]133;P;Cwd=<path>\e\` | Directory sync for the sidebar panel |

---

## 5. Non-Negotiable Principles and Constraints

- **Strict memory budget (< 80 MB RAM):** the idle executable runs between 30 and 50 MB; absolute ceiling 80 MB. Blocks outside the viewport are virtualized.
- **Zero web dependencies:** Electron, WebKitGTK, Tauri, or embedded Chromium are forbidden.
- **Linux first & Zsh first:** development and initial support exclusively on modern Linux distributions on Wayland/X11 with the Zsh shell.
- **Inviolable clean architecture:** the domain knows nothing about GPUI, sockets, or the OS; everything connects through Ports and Adapters.

---

## 6. Technology Stack

| Role | Crate / Technology |
| :--- | :--- |
| Language | Rust (Edition 2021+) |
| UI and GPU | `gpui` (Zed) — Vulkan / OpenGL on Linux |
| PTY and child processes | `portable-pty` |
| ANSI/VT100 Parser | `vte` |
| File monitoring | `notify` |
| Local history (Phase 5+) | `rusqlite` |
| Async management | `smol` or GPUI's native executor |

---

## 7. Roadmap (Summary)

| Phase | Key Deliverable |
| :--- | :--- |
| 0 | Cargo workspace + basic GPUI window on Wayland (sustained 143.8 Hz compositor pacing; idle PSS ≈ 70 MB attributed floor, < 80 MB ceiling verified in Phase 6; X11 verification deferred) |
| 1 | PTY subsystem (`portable-pty` + background reader + `vte`) |
| 2 | OSC 133 semantic protocol + first functional visual block |
| 3 | GUI input editor + sticky header |
| 4 | Immersive mode (alternate screen) |
| 5 | Tools Panel (sidebar file explorer) |
| 6 | Performance and memory audit (heaptrack, virtualization, < 80 MB) |

---

## 8. Development Environment (Docker Sandbox)

To guarantee full parity with GPU drivers (Vulkan) and Wayland without polluting the Fedora host installation, the entire environment is containerized (`compose.yml`, `dev.Dockerfile`, `mise.toml`):

```bash
mise run up      # start the container in the background (docker compose up -d)
mise run shell   # interactive terminal inside the container (zsh)
mise run run     # build and run the native window (cargo run)
mise run clean   # destroy the environment and free space
```

Standard verification inside the container: `cargo check && cargo clippy -- -D warnings && cargo test`.
