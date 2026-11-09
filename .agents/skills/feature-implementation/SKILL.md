---
name: feature-implementation
description: Implement a new feature end-to-end (Presentation component → Application use case → Domain entity/port → Infrastructure adapter) while preserving the Clean Architecture layer boundaries and minimizing unnecessary exploration. Use when the user asks to add new functionality such as a UI component, a block capability, an OSC semantic event, or a port implementation. Do not use for fixing existing behavior — that is bug-investigation.
---

# Feature Implementation Skill

## Purpose

Define the workflow agents must follow when implementing a new feature in the
native terminal engine.

The objective is to add functionality while preserving existing
architecture (Clean Architecture / Ports & Adapters), minimizing unnecessary
exploration and avoiding scope expansion.

# Execution Workflow

## 1. Understand the Requirement

Before reading source code:

Identify:

- Requested functionality.
- Objective.
- Explicit constraints (memory budget, headless testability, layer boundaries).
- Expected behavior.
- Existing context provided by the user.

Do not start repository exploration without a purpose.

---

# 2. Read Internal Documentation First

Before inspecting implementation details, review the relevant internal
documentation.

Priority:

1. Capability specifications: `knowledge/specs/<capability>/`.
2. Project knowledge: `knowledge/architecture.md`,
   `knowledge/conventions.md`, `knowledge/project-map.md`.
3. ADRs when a settled decision applies: `knowledge/governance/decisions/`.

Read only documents related to the requested functionality. Do not load all
documentation by default.

If the matching specification includes an **Acceptance Criteria** section,
treat it as the implementation contract for this feature — verify against it
in the Completion Report (§ 8).

---

# 3. Define the Implementation Scope

Before modifying code, identify:

- Existing behavior.
- Missing behavior.
- Required changes.
- Affected layers (use the Change Mapping table in
  `knowledge/architecture.md`).
- Expected files (entry points in `knowledge/project-map.md`).

Create a technical plan containing:

- Implementation approach.
- Files that will change.
- Expected execution flow through the two lifecycles (outbound input,
  inbound PTY output).
- Any relevant risks or ambiguities.

If the scope is not clear, ask the minimum required question before
continuing.

---

# 3.5. Delegated Retrieval (subagent)

Once § 3 has fixed the list of affected files/layers, the mechanical
retrieval may be delegated to a fast subagent:

- exact file paths from the technical plan (domain entity, port trait,
  application service, infrastructure adapter, presentation component);
- the sibling module used as the pattern reference, if one exists.

The subagent returns raw contents/relevant excerpts only — no
interpretation, no decisions about what to change.

Per `knowledge/governance/policies.md` § 13, delegation also covers
mechanical discovery searches (`rg`/`fd` sweeps) and verification loops
(`cargo check`/`clippy`/`test`). None of these belong on the main thread
during discovery — hand the exact command(s) to the subagent and have it
report back pass/fail or the matched locations.

Cap: if the plan lists more than 10 files, the subagent stops at 10, reports
what it found, and asks before reading further (`policies.md` § 16). Do not
let it silently keep going.

Interpreting this content and deciding what to implement stays on the main
thread — the subagent never writes implementation code (`policies.md` § 13
boundary, § 15 scope).

---

# 4. Source Code Navigation

Inspect only the layers required by the change. Do not review every
architectural layer automatically.

Follow this order for a new feature inside the engine:

```text
Specification
      ↓
Presentation component (if visual)  ├──  updates state via
      ↓                             │
Application use case / service ─────┤
      ↓                             │
Domain entity / port / event variant │
      ↓                             │
Infrastructure port implementation   ┴── events flow back inbound
```

Task-specific flows (from `knowledge/architecture.md` § Change Mapping):

- **New OSC semantic code:** parser adapter in `src/infrastructure/parser/`
  → `SemanticEvent` variant in `src/domain/semantic.rs` → handler in the
  application service → (optional) presentation reaction.
- **Persistence (Phase 5+):** implement `HistoryRepository` under
  `src/infrastructure/persistence/`; the Domain trait stays identical.
- **Full-screen handling:** alt-screen escape interception lives in
  infrastructure; the `DisplayMode` decision lives in Domain.
- **Cwd sync:** `133;P;Cwd=` marker → `DirectoryChanged` event → Tools Panel
  refresh.

If the feature depends on a new dependency, that is a separate approval
(`knowledge/governance/policies.md` § 5) — resolve it before implementing.

---

# 5. Implementation Rules

During implementation:

- Preserve existing architecture.
- Follow existing project patterns (`knowledge/conventions.md`).
- Keep changes focused.
- Reuse existing components when possible.
- Avoid unrelated improvements.
- Avoid introducing new dependencies.
- Avoid architectural changes.

The goal is the smallest correct implementation.

When editing an existing file, use exact-match edits — do not rewrite the
whole file unless the change touches most of it (`policies.md` § 14).

---

# 6. Layer Implementation Rules

Full layer responsibilities and boundaries:
`knowledge/architecture.md` § Layer Responsibilities — not repeated here.

Rules specific to implementation:

- Domain: pure types and ports only; no `gpui`, `portable_pty`, or OS
  imports; 100% headless-testable.
- Application: coordination and use cases only; no GPUI primitives, no direct
  syscalls — always through ports.
- Infrastructure: OS adapters only; never block the GPUI thread; translate OS
  events into Domain types (e.g. `SemanticEvent`).
- Presentation: view composition and event routing only; keystrokes and
  terminal actions go through Application use cases, never straight to the PTY.

---

# 7. Scope Control

If a better solution is discovered outside the requested functionality:

Do not implement it automatically.

Instead:

1. Explain the improvement.
2. Explain why it may be valuable.
3. Continue only with the approved scope.

---

# 8. Completion Report

After implementation, provide:

- Summary of implemented behavior.
- Files modified.
- Important decisions made.
- Validation performed.
- Remaining considerations.

Before finishing:

- Run the mandatory verification (`AGENTS.md` § 8): `cargo check`,
  `cargo clippy -- -D warnings`, `cargo test` — all must pass with zero
  warnings.
- If the specification has Acceptance Criteria, list each with pass/fail
  against the implementation. If any diverges, report the divergence and ask
  before updating the spec (`policies.md` § 10) — do not silently drift.
- If this implementation was driven by a `create-specification` full SDD
  cycle (a `tasks.md` exists for the capability under
  `knowledge/specs/<capability>/`), recommend running the `drift-audit` skill
  if there is any chance the shipped spec no longer matches the code — never
  edit the spec silently (`policies.md` § 10).
- Before running any command beyond the existing verification set (installing
  crates, generating files, touching Docker), check
  `knowledge/governance/policies.md` § 5, § 6 and § 7 — request approval
  unless it's an existing command run as-is.

Do not include unrelated recommendations as completed work.
