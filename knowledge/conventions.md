# Coding Conventions

## Purpose

**Question answered by this document:**

> How should agents write and structure code in this project?

This document defines the coding standards and implementation preferences to
follow when creating or modifying Rust code in this repository.

The goal is not to minimize lines of code; it is to create code that is easy to
understand, maintain, and modify over time.

---

# Coding Philosophy

Main priority order:

```text
Maintainability
      ↓
Readability
      ↓
Correctness
      ↓
Reusability
      ↓
Optimization
```

A technically clever solution loses to a boring, obvious one unless the hot
path demands otherwise (and the hot path is documented in
`knowledge/architecture.md` § Architecture Philosophy).

---

# Formatting & Linting

- **rustfmt is the arbiter.** Keep the default `cargo fmt` profile; never
  hand-adjust formatting. Any formatting disagreement ends with
  `cargo fmt --check` passing.
- **`cargo clippy -- -D warnings` must pass.** Zero-warning policy: fix the
  lint, or restructure; do not silence with `#[allow]` without an inline
  justification comment explaining why the default is wrong here.
- Run both inside the container (`AGENTS.md` § Standard Commands).

---

# Module Layout

- One component = one module, placed in the layer directory that owns it
  (`knowledge/architecture.md` § Project Organization).
- Layer dirs group by capability only when the component list grows
  (`infrastructure/parser/`, `presentation/components/`).
- Cross-cutting utilities live in `src/common/` — never duplicated per layer.
- A module's `pub` surface should be as narrow as the application layer needs;
  hide internals with `pub(crate)` when possible.

---

# Naming

- Files and modules: `snake_case` (`block_card.rs`, `session_service.rs`).
- Types, traits, enums: `PascalCase`; traits are nouns describing contracts
  (`PtyPort`, `HistoryRepository`), not verbs (`PtyHandler`).
- Constants: `SCREAMING_SNAKE_CASE`.
- State enums use lifecycle nouns (`BlockStatus::{WaitingForInput, Running,
  Completed, Failed}`), not imperative verbs.
- Domain events end in the shell marker they represent (`PromptStart`,
  `CommandFinished`), mirroring OSC 133 naming from the spec.

---

# Error Handling

- Single application error type: `TerminalError` in `src/common/errors.rs`.
- Fallible APIs return `Result<T, TerminalError>`; propagate with `?`.
- **No `unwrap()`/`expect()` in production code.** Tests may use them only
  where the invariant is locally obvious.
- Never swallow errors: match-or-propagate. Conversion from infrastructure
  failures (IO, PTY, parse) into `TerminalError` happens at the infrastructure
  boundary, not inside domain logic.
- Domain functions stay infallible where the state machine forbids the error
  in the first place (model the impossible state away with types).

---

# Comments & Documentation

- `///` doc comments on all public domain traits/ports and on any function
  whose contract is not obvious from its signature.
- Comments explain **why**, not what. Non-obvious invariants (buffer caps,
  thread boundaries, OSC marker assumptions) must leave a comment.
- No commented-out code, no `TODO` without a reported issue behind it.

---

# Memory & Performance Conventions

These exist to protect the < 80 MB RAM budget (`knowledge/roadmap.md` § Phase 6):

- Terminal output is never accumulated as an unbounded `String`/`Vec`.
  Output buffers are bounded, segmented, or virtualized; off-screen content
  may be dropped or compressed.
- Zero allocations on the critical rendering path; prefer `&str`/borrowed
  spans over constructed `String`s when the data is already owned upstream.
- Thread boundaries: PTY reads, file watching, and child-process waits live in
  infrastructure workers; results reach GPUI through executor channels or
  events — never by blocking a render thread.
- A dependency is a warning, not a solution: before reaching for a crate to
  solve a small problem, check `AGENTS.md` § STOP (approval required) and the
  low-memory filter.

---

# Testing

- `cargo test` must run headless — no GPU window, no PTY process in unit tests.
- Unit tests live colocated in the same file under `#[cfg(test)]` unless a
  module's tests grow into a dedicated `tests/` submodule.
- State machines (`block.rs`, `semantic.rs`) must be testable by injecting
  `SemanticEvent`s directly — no shell required.
- Parser tests use raw byte fixtures for escape sequences; do not construct
  ANSI strings ad hoc inside assertion logic.

---

# Shell Protocol Hygiene

- The representation consumed by Application/Domain for shell semantics is
  `SemanticEvent` (`src/domain/semantic.rs`) — nothing else.
- Escape sequences are parsed once, in `src/infrastructure/parser/`; their
  byte literals (`\x1b]133;...`) must not appear in application or
  presentation code.
- Zsh hooks (`precmd`/`preexec`) are part of the protocol surface
  (`knowledge/roadmap.md` § OSC 133 Semantic Protocol); changes to them follow the same Change Mapping as
  parser changes.

---

# Known Gotchas

Empty by design — findings from bug investigations and code reviews get
recorded here as they are discovered. Keep entries to one line each:

> Gotcha → consequence → prevention.
