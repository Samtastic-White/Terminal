---
name: bug-investigation
description: Investigate an existing defect end-to-end — reconstruct the execution flow, form a hypothesis, confirm the root cause in source code, then apply the smallest correct fix. Use when the user reports a bug, unexpected behavior, or asks to debug/fix something in the terminal engine (PTY, rendering, parsing, blocks) that used to work. Do not use for building new functionality — that is feature-implementation.
---

# Bug Investigation Skill

## Purpose

Define the workflow agents must follow when investigating and fixing existing
defects.

The objective is to identify the real cause of a problem, apply the smallest
correct fix, and avoid unrelated changes.

# Investigation Workflow

## 1. Understand the Reported Problem

Before reading source code:

Identify:

- Expected behavior.
- Current behavior.
- Difference between both states.
- User-provided context.
- Possible affected lifecycle (outbound input flow, inbound PTY/output flow) —
  see `knowledge/architecture.md` § Execution Lifecycles.

Do not start repository exploration without a clear investigation objective.

---

# 2. Reconstruct the Execution Flow

Before searching files, understand the expected system flow. Full lifecycles:
`knowledge/architecture.md` § Execution Lifecycles — not repeated here.

Determine where the expected behavior could diverge from the current
implementation. For terminal-engine bugs, classify by lifecycle leg first:

- Input leg: `InputEditor → SessionService → Block → PtyPort`.
- Output leg: `PTY reader thread → parser (ANSI/OSC 133) → BlockService →
  BlockStatus transition → presentation render`.

---

# 3. Form a Hypothesis

Before making changes, create a technical hypothesis when necessary.

Example:

> "The block stays Running after the shell exits the command because the parser never emits `CommandFinished` when the `133;D` marker arrives with a nonzero exit code."

The hypothesis must be verified against the source code.

Do not modify code based only on assumptions.

---

# 4. Context Acquisition

Acquire only the context required to validate the hypothesis.

Priority:

1. Relevant capability specs: `knowledge/specs/<capability>/`.
2. Relevant project knowledge: `knowledge/architecture.md`,
   `knowledge/project-map.md`, `knowledge/conventions.md`.
3. Specific source files.

Once the hypothesis identifies which of these files could confirm or reject
it, the raw retrieval may be delegated to a fast subagent — it fetches and
returns contents/excerpts, flags missing files, and makes no judgment about
root cause (`knowledge/governance/policies.md` § 13).

Cap: 10 files per turn (`policies.md` § 16). If confirming the hypothesis
needs more, the subagent stops, summarizes what it found, and asks before
continuing.

Confirming or rejecting the hypothesis against this content (§ 5) is judgment
and stays on the main thread — never delegated.

Avoid:

- Reading the entire repository.
- Exploring unrelated modules.
- Searching without a defined reason.

---

# 5. Root Cause Identification

Before implementing a fix, identify:

- The exact location of the defect.
- Why the current behavior occurs.
- Why the proposed change resolves it.

The goal is to fix the cause, not only the visible symptom. A fix inside
Infrastructure that paper-overs a missing Domain state transition is a
symptom fix; trace the state machine to its legitimate layer first
(`knowledge/architecture.md` § Change Mapping).

---

# 6. Fix Strategy

When the root cause is confirmed:

- Apply the smallest correct change.
- Preserve existing architecture and layer boundaries.
- Preserve existing behavior outside the defect.
- Avoid unrelated refactoring.

Do not improve surrounding code unless it is required for the fix.

---

# 7. Additional Findings

During investigation, other issues may appear.

Examples:

- Possible refactors.
- Naming improvements.
- Architecture improvements.
- Missing edge-case handling (memory budget, alt-screen, PTY death).

Do not implement them automatically.

Report them separately as recommendations — and record genuinely general
lessons as one-line entries in `knowledge/conventions.md` § Known Gotchas
only if the user approves (`knowledge/governance/policies.md` § 10).

---

# 8. Validation

After applying the fix:

Validate that:

- The original problem is resolved.
- Existing flows remain unaffected.
- No unnecessary files were modified.

Run the existing verification commands in the container (`AGENTS.md` § 3 and
§ 8): `cargo check`, `cargo clippy -- -D warnings`, `cargo test`. Installing
dependencies or generating new artifacts requires approval first
(`knowledge/governance/policies.md` § 6, § 7).

Report:

- Root cause.
- Files changed.
- Fix applied.
- Validation performed.
- Additional observations.

---

# Core Principle

A bug fix is successful when the smallest necessary change restores the
expected behavior without expanding the scope.
