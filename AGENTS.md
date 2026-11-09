# Native Terminal Engine — Agent Instructions

Single source of truth for agent behavior in this repository. It applies to any
CLI agent or model working here (OpenCode, Claude Code, Codex, Gemini CLI,
Cursor, ...). Tool-specific pointer files (e.g. `CLAUDE.md`) must only load
this file, never restate it.

Project knowledge lives in `knowledge/`. This file holds operative rules only.

---

## Context Map — load tiers

| Tier | Path | Loaded when |
| --- | --- | --- |
| 0 — auto | this file | every session, by every conforming tool |
| 0 — auto | `.agents/skills/` | advertised per task; body loaded on demand |
| 1 — on demand | `knowledge/roadmap.md` | product intent or pending scope unclear |
| 1 — on demand | `knowledge/architecture.md` | touching layers, lifecycles, Change Mapping |
| 1 — on demand | `knowledge/conventions.md` | writing or modifying Rust code |
| 1 — on demand | `knowledge/project-map.md` | navigating the repo, multi-file discovery |
| 1 — on demand | `knowledge/governance/` | approvals, forbidden actions, ambiguity rules |
| 1 — on demand | `knowledge/governance/decisions/` | asking why X over Y (ADRs) |
| flow | `knowledge/specs/<capability>/` | spec-driven development (create-specification) |
| 2 — never auto | `knowledge/archive/` (if introduced) | excluded from scans by convention |

Read only what the current task names. When a tier-1 document answers the
question, stop reading. Source code is the final implementation authority.

---

## 1. Environment & Setup

All development and compilation happen inside the isolated Fedora container for
GPU/Vulkan parity without polluting the host OS.

```bash
mise run up      # start container in background (or: docker compose up -d)
mise run shell   # interactive shell inside the container (zsh)
mise run clean   # stop and clean (or: docker compose down --rmi all -v)
```

Once inside the container (`/workspace`), verify with `cargo check` before
declaring any task complete.

## 2. Stack

- **Language:** Rust (2021 edition or newer).
- **GUI engine:** GPUI (Zed) with direct Vulkan/Wayland rendering.
- **PTY controller:** `portable-pty`; target shell `/bin/zsh` with semantic OSC 133 markers.
- **Parser / FS / persistence:** `vte`, `notify`, `rusqlite` (Phase 5+ only).
- **Runtime:** Fedora 41 container (Docker Compose + user UID mapping).

## 3. Standard Commands

Run verification before completing any task:

| Action | Command (inside container) | Host shortcut |
| --- | --- | --- |
| Check compilation | `cargo check` | `docker compose exec dev cargo check` |
| Lint (strict) | `cargo clippy -- -D warnings` | `docker compose exec dev cargo clippy -- -D warnings` |
| Format check | `cargo fmt --check` | `docker compose exec dev cargo fmt --check` |
| Unit tests | `cargo test` | `docker compose exec dev cargo test` |
| Run | `cargo run` | `mise run run` |

## 4. STOP — Non-negotiable Rules

Before performing any of these, stop and ask for explicit confirmation. Never
implement a "smaller, low-risk" version without asking first:

1. **General scope clause.** Implement only what is strictly necessary. No
   "while I'm here" refactors, unsolicited renames, or styling changes in
   untouched files.
2. **Dependencies.** Never add, remove, or update crates in `Cargo.toml`
   without prior approval. Every crate must pass the strict low-memory filter
   (< 80 MB RAM budget).
3. **Architectural violations.** Never import `gpui`, `portable_pty`, or
   infrastructure code into `src/domain/`. Layer boundaries in
   `knowledge/architecture.md` are inviolable.
4. **Deleting code or files.** Never remove existing files, functions,
   modules, or tests without explicit instruction.
5. **Environment & container config.** Never edit `dev.Dockerfile`,
   `compose.yml`, or `.env` unless explicitly tasked.
6. **Git operations.** Never run destructive or state-changing git commands
   (`git reset --hard`, `git push --force`, `git clean -f`, commits).
7. **Reference repositories.** The sibling repos `../warp-terminal/`,
   `../wave-terminal/`, and `../zed-gpui/` are analysis-only. Never copy,
   transcribe, or closely translate their code into this repository —
   Warp is predominantly AGPL-3.0, and a single transcription would
   contaminate the project's own licensing. Knowledge flows through
   specs, notes, and reimplementation from principles. Provenance guard:
   `rg -i "denver technologies|license-alacritty|warp_terminal|waveterm" src/`
   must stay empty at all times.

Full approval boundaries, forbidden actions, and the explain → justify → wait
procedure: `knowledge/governance/policies.md`.

## 5. Task Classification

Classify the request before inspecting source code:

- **Bug fix** — broken behavior in PTY, rendering, or parser flows.
- **New feature** — new block, UI component, or semantic capability.
- **Refactor** — reorganizing within the same layer, no behavior change.
- **Architecture** — cross-layer contracts (ports/traits in Domain).
- **Performance / memory audit** — allocations, idle RAM, buffer virtualization.
- **Shell protocol (OSC 133)** — semantic escape parsing or Zsh hooks.
- **Documentation** — `knowledge/roadmap.md`, `knowledge/architecture.md`, `README.md` / `README.es.md`.

## 6. Context Acquisition Order

Read in this strict sequence; stop as soon as the question is answered:

1. User prompt (intent).
2. This file (operative rules).
3. `knowledge/roadmap.md` (product intent / backlog).
4. `knowledge/architecture.md` (layer boundaries, Change Mapping).
5. Target domain port/entity (`src/domain/`).
6. Smallest implementation file directly addressing the task.
7. Repo-wide search (`rg`/`fd`) — last resort, with a hypothesis.

## 7. Exploration Rules

- **Symbol first.** Use `rg`, `fd`, or LSP to locate structs/traits/functions —
  never read whole directory trees.
- **Trust the compiler.** When in doubt about a type or trait, run
  `cargo check` in the container instead of guessing APIs.
- **Stop rule.** If exploration touches more than 5 unrelated files without
  answering the question, stop, summarize, and ask.
- **Budget.** File-read caps and delegation discipline:
  `knowledge/governance/policies.md` § 13 and § 16.

## 8. Implementation & Verification Workflow

1. **Isolate the change.** Locate the owning layer via the Change Mapping in
   `knowledge/architecture.md`.
2. **Apply the minimal diff.** Smallest correct idiomatic Rust change.
3. **Verify concurrency & memory.** No blocking I/O (PTY reads, file queries)
   on the GPUI render thread; buffers capped or virtualized — never
   accumulate unbounded raw byte strings.
4. **Mandatory pre-completion verification.** Run and pass:

```bash
cargo check
cargo clippy -- -D warnings
cargo test
```

No broken windows: leave the codebase with zero warnings and zero failing
tests. Do not add crates without approval (STOP rule 2); do not alter
`.env` or Docker configuration unless asked (STOP rule 5).

## 9. Architectural Invariants

Details and rationale: `knowledge/architecture.md`. The hard edges:

- `src/domain/` NEVER imports `gpui`, `portable_pty`, `rusqlite`, or OS crates.
- Never block the UI thread: PTY reads, file queries, and child-process waits
  run on background workers or executor channels.
- Presentation never touches PTY directly — keystrokes route through
  `src/application/` use cases.
- Memory < 80 MB: outputs are virtualized/segmented; zero WebViews, zero
  Electron, zero Chromium.

## 10. Shell Protocol (OSC 133)

The execution model depends on semantic markers injected into Zsh:

| Marker | Meaning | Engine effect |
| --- | --- | --- |
| `\e]133;A\e\` | Prompt start | terminal prepares for new input |
| `\e]133;B\e\` | Prompt end | user typing / input mode |
| `\e]133;C\e\` | Command executed | stdout begins; active block created |
| `\e]133;D;<code>\e\` | Command finished | block completed with exit code |
| `\e]633;P;Cwd=<path>\e\` | Cwd notification (VS Code convention) | Tools Panel refresh |
| `\e]633;E;<cmd>\e\` | Executed command line (VS Code convention) | Block header text; payload read verbatim (take-rest after `633;E;`) |

New escapes: update `src/infrastructure/parser/`, add the variant in
`src/domain/semantic.rs`, handle it in `src/application/`.

The Rust state machine is tolerant by design: `D` without prior `C`
(cold start, Ctrl+C at the prompt) and `A` while running are ignored;
shell termination without `D` (`exit`, crash, PTY EOF) finalizes the
running block as `Interrupted`. A bare `133;P` (continuation prompt in
the semantic-prompts standard) or a `133;P` with non-`Cwd` options is
silently ignored — the directory rides on `633;P;Cwd=` instead. Full
transition table: `knowledge/roadmap.md` § Block State Machine.

## 11. Communication

Ask questions only when requirements are truly ambiguous or a STOP rule fires.
When asking, state:

- what is ambiguous or detected as an issue;
- why it matters (architecture/memory/performance impact);
- the specific decision or approval needed from the user.

Documentation language: repository documentation is written in English and
every document is a single canonical English file, except the root README
pair. `README.md` is the canonical source of truth; `README.es.md` is its
Spanish translation and must follow it, never the reverse. Converse in the
user's language.
