# Terminal Engine Architecture

## Purpose

**Question answered by this document:**

> How is the native terminal engine organized, and where should an engineer implement each type of change?

This document describes the architectural model of the native terminal. It
guides development while maintaining strict layer isolation, zero web-engine
dependencies, and a memory footprint under 80 MB.

It does not describe every file or implementation detail. The Rust source code
is always the source of truth.

**Read this file when:** you need to trace an end-to-end execution lifecycle
(user input to PTY, or PTY output to GPUI rendering), check the Change Mapping
table before editing files, understand layer boundaries, or review the Common
Mistakes checklist before committing code.

---

# Architecture Philosophy

The project follows **Clean Architecture (Hexagonal / Ports & Adapters)** in
native Rust. Each layer has a single responsibility and must not invade the
responsibilities of another layer.

Core objectives:

- **Strict isolation:** UI frameworks (GPUI) and OS primitives (PTY/Vulkan) are
  volatile details; the Domain model is pure and stable.
- **Predictable performance:** zero allocations on critical rendering paths;
  memory strictly under 80 MB.
- **Testability:** 100% of the block lifecycle and semantic parsing must be
  unit-testable in headless CLI mode without spawning GPU windows.
- **Consistency:** maintainability and idiomatic Rust over clever one-line hacks.

---

# Project Organization

Global architectural layers, with internal module grouping by capability:

```text
src/
├── presentation/      # Driving Adapters: GPUI views, components, layouts, themes
├── application/       # Orchestration: Services, use cases, session management
├── domain/            # Core: pure entities, state machines, traits (Ports)
├── infrastructure/    # Driven Adapters: PTY, parser, file watcher, SQLite
└── common/            # Cross-cutting: Result/Error types, shared primitives
```

---

# Execution Lifecycles

The engine operates on two distinct, asynchronous execution cycles.

## 1. User Input Flow (Outbound / Driving)

Triggered by user keyboard or mouse interaction:

```text
Keyboard / Mouse Event
        │
        ▼
Presentation (InputEditor / GPUI View)
        │
        ▼
Application (SessionService::execute_command)
        │
        ▼
Domain (Block::new, Status::Running, Prompt validation)
        │
        ▼
Infrastructure (PtyAdapter / Write bytes to Master PTY)
        │
        ▼
Child Process (/bin/zsh)
```

## 2. Terminal Output & Semantic Flow (Inbound / Reactive)

Triggered by bytes emitted from the child shell process:

```text
Child Process (/bin/zsh)
        │
        ▼
Infrastructure (portable-pty background reader thread)
        │
        ▼
Infrastructure (OSC 133 Parser / ANSI Lexer)
        │
        ▼
Application (BlockService::handle_semantic_event)
        │
        ▼
Domain (Transition BlockStatus: Running ──► Completed { exit_code })
        │
        ▼
Presentation (GPUI Model::update ──► cx.notify())
        │
        ▼
GPUI Element Tree (renders dirty BlockCard directly to GPU)
```

---

# Layer Responsibilities

## 1. Domain Layer (`src/domain/`)

The innermost layer: pure business logic, state machines, and Port traits.

Rules:

- MUST NEVER import `gpui`, `portable_pty`, `rusqlite`, or external system
  libraries.
- Must compile fast and be 100% unit-testable in isolation (`cargo test`).

Components:

- `Block`: entity representing a command card (ID, command, working directory,
  output buffer, status).
- `BlockStatus`: enum tracking lifecycle (`WaitingForInput`, `Running`,
  `Completed`, `Failed`).
- `SemanticEvent`: domain representation of shell markers (`PromptStart`,
  `CommandStart`, `CommandExecuted`, `DirectoryChanged`).
- Ports (traits):
  - `PtyPort`: contract for writing to and resizing the terminal process.
  - `HistoryRepository`: contract for persisting and querying previous executions.
  - `FileSystemPort`: contract for reading directory structures.

## 2. Application Layer (`src/application/`)

Orchestrates business use cases. Coordinates between the Domain and
Infrastructure ports.

Rules:

- MUST NEVER contain GPUI rendering primitives (`div`, `render`, `WindowContext`).
- MUST NEVER invoke OS syscalls directly (always use Ports).

Components:

- `SessionService`: manages child shell lifecycle, cwd sync, active processes.
- `BlockService`: manages active blocks, sticky headers, history limits,
  block collapsing.
- `HistoryService`: queries historical executions for search and visual history panels.

## 3. Infrastructure Layer (`src/infrastructure/`)

Implements the Driven Ports defined by the Domain. Manages real-world I/O and
OS communication.

Rules:

- MUST NEVER contain business domain rules.
- MUST NEVER block the main UI thread. PTY reading and disk queries run
  asynchronously or on worker threads.

Components:

- `pty/`: implements `PtyPort` using `portable-pty` to control `/bin/zsh`.
- `parser/`: parses ANSI/VT100 escape sequences and translates OSC 133
  sequences into Domain `SemanticEvent`s.
- `persistence/`: implements `HistoryRepository`:
  - `in_memory.rs`: fast, zero-dependency history for initial phases.
  - `sqlite.rs`: embedded SQLite (`rusqlite`) for fast, low-RAM indexing.
- `filesystem/`: reads directory trees for the Tools Panel using `notify`
  and `std::fs`.

## 4. Presentation Layer (`src/presentation/`)

The Driving Adapter. Built entirely with Zed's native GPUI framework.

Rules:

- MUST NEVER contain terminal state logic or PTY operations.
- Translates GPUI events (keystrokes, mouse drags, scrolls) into calls to the
  Application layer.
- Observes GPUI `Model<T>` and re-renders elements efficiently on the GPU.

Components:

- `window.rs`: main window bootstrap, layout, split-pane orchestrator.
- `components/block_card.rs`: visual card container (borders, exit-code
  badges, copy buttons).
- `components/sticky_header.rs`: pinning logic for headers during long output
  scrolling.
- `components/input_editor.rs`: native multi-line GUI input editor
  (substituting ZLE).
- `theme/`: design tokens, colors, paddings, font configurations.

## 5. Common Layer (`src/common/`)

Shared foundational utilities across layers.

Components:

- `errors.rs`: application-wide `Result<T, TerminalError>`.
- `id.rs`: monotonic or UUID generation for blocks.

---

# Dependency Direction

Dependencies point strictly inward toward the Domain.

```text
Presentation ──────► Application ──────► Domain
                           ▲
                           │
Infrastructure ────────────┘
```

- Domain depends on nothing.
- Application depends only on Domain.
- Presentation depends on Application (and Domain types).
- Infrastructure implements traits from Domain.

---

# Change Mapping

Identify which layers are affected by standard tasks before editing:

| Task | Presentation | Application | Domain | Infrastructure |
| --- | --- | --- | --- | --- |
| Support new OSC semantic code | — | Update handlers | Add `SemanticEvent` variant | Update parser |
| Change card UI / hover buttons | Update `block_card.rs` | — | — | — |
| Add command duration badge | Render badge in UI | Pass duration | Add duration to `Block` | Track start/end timestamp |
| Switch RAM → SQLite history | — | — | (trait stays identical) | Implement `SqliteHistory` |
| Handle full-screen app (htop/vim) | Switch to `AltScreenView` | Toggle session mode | Add `DisplayMode::AltScreen` | Intercept `\x1b[?1049h` |
| Cwd auto-sync in Tools Panel | Update tree view | Sync working dir | Add `DirectoryChanged` event | Intercept `OSC 133;P;Cwd=` |

---

# Common Mistakes to Avoid

1. **Blocking the UI thread:** never read the PTY inside a GPUI render loop.
   Use a background thread or GPUI executor channels.
2. **UI references in Domain:** never import `gpui::*` or use graphic
   colors/coordinates inside `domain/`.
3. **Bypassing the Application layer:** never send keystrokes directly from
   `input_editor.rs` to `pty_adapter.rs`; route through `SessionService`.
4. **Treating blocks as plain text:** do not store command output as a raw
   monolithic `String`; store structured spans or line matrices.
5. **Memory leaks on infinite output:** commands with millions of log lines
   must drop or truncate off-screen content; use virtualization to honor the
   < 80 MB RAM budget.
6. **Hardcoded escape sequences in views:** UI components must never parse
   ANSI strings; parsing is the sole responsibility of the Infrastructure
   parser.

---

# Navigation Strategy

When navigating or implementing features:

1. Start with the Domain model: check `src/domain/block.rs` for existing
   states and data.
2. Trace the Port: locate the trait in `domain/` to understand the contract.
3. Inspect the Infrastructure implementation: how the OS/PTY fulfills the contract.
4. Connect via the Application service: verify orchestration rules.
5. Render in Presentation: bind the state to GPUI elements.
