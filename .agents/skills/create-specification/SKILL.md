---
name: create-specification
description: Run this project's Spec-Driven Development flow for a business capability. For a new capability or an approved behavior change not yet implemented, produce a gated pre-implementation contract (proposal -> specification -> optional design -> tasks) under knowledge/specs/<capability>/; no production code is written before the Tasks gate is approved. For already-shipped, stable behavior, write or update the lightweight specification.md only. Use when the user asks to spec out, plan, or document a capability, or before implementing a new/changing business capability. Do not use to document implementation internals — source code remains the implementation source of truth.
---

# Create Specification Skill

## Purpose

Define the workflow agents must follow to run Spec-Driven Development (per
specdriven.ai) for this project's business capabilities: capture intent before
code exists, gate explicitly before implementation, and keep the
specification as the capability's living source of truth afterward.

The objective is to document system behavior, business rules and decisions
without replacing source code as the implementation authority — and, for new
or changing capabilities, to produce the contract that gates implementation.

---

# Two Modes

**Full SDD cycle** — the capability is new, or an approved behavior change is
not yet implemented. Produces, in order, `proposal.md` -> `spec.md` ->
`design.md` (conditional) -> `tasks.md` under `knowledge/specs/<capability>/`,
each gated before the next is written. No production code before the Tasks
gate (Gate 3) is approved.

**Lightweight documentation** — the behavior is already shipped and stable;
nothing is pending implementation. Write or update `spec.md` only. No
proposal/design/tasks, no gates — there is nothing to gate.

Determine the mode from the request before doing anything else: if the code
for the described behavior does not exist yet, or an approved change to
existing behavior hasn't been implemented, use the full cycle.

---

# Specification Objective

A specification must answer:

> What does this capability do, why, and what rules govern its behavior?

`spec.md` describes:

- Business capability and user stories (the intent — who wants what, and why).
- Business rules.
- Edge cases and error handling.
- Non-functional requirements, when applicable.
- Access control.
- Relationships with other capabilities.
- Known limitations and open questions.

It does not describe:

- Every implementation detail, line-by-line source behavior, or internal
  algorithms.
- Temporary technical decisions unless they affect behavior.

Source code remains the implementation source of truth for _how_.

---

# Information Sources

Use available information from:

1. User requirements.
2. Existing specifications (`knowledge/specs/`).
3. Existing project knowledge (`knowledge/`).
4. Source code implementation.

Do not rely exclusively on source code when business context is required. Do
not invent business rules that cannot be confirmed — see Ambiguity Handling.

---

# Context Acquisition

Recover only what the artifact you are about to write needs. Do not
front-load full codebase context before the cheapest gate has passed —
investing in deep context (source code, design-level detail) only pays off
once the work has survived the earlier, cheaper gate.

**Before `proposal.md`** (full cycle only):

1. If `knowledge/specs/<capability>/spec.md` already exists, read it in full —
   it is the current source of truth and must not be contradicted.
2. `knowledge/project-map.md` only if the capability boundary is unclear.

Do not read implementation source code at this stage — the proposal is
business scope, not implementation detail.

**Before `spec.md`** (after Gate 1):

1. Related existing specs — only capabilities plausibly connected
   (`[[wikilinks]]`), not all of `knowledge/specs/`.
2. `knowledge/conventions.md` / `knowledge/architecture.md` — only if a rule
   or layer boundary is unclear.
3. Source code — the mechanical retrieval may be delegated to a fast
   subagent; the main thread reads only the filtered result. Cap: 10 files
   per turn (`knowledge/governance/policies.md` § 16); stop and ask before
   reading more.

**Before `design.md`** (only when this artifact is created):

1. Patterns from sibling capabilities at the affected layer.
2. Current state model / relevant entities, current dependencies — only the
   ones the design decision touches.

Same 10-file cap; on hitting it, stop, summarize, and ask before continuing.

**Before `tasks.md`:**

No new discovery pass. Build directly on what was already gathered for
`spec.md` / `design.md`. Only consult `knowledge/project-map.md` to pin exact
file paths per task.

Distilling business rules and writing specification/design/task text is
judgment — never delegated (`knowledge/governance/policies.md` § 13
boundary). Only the mechanical retrieval is delegated.

---

# Artifacts

## `proposal.md` — Specify (outline)

One page, cheap to write and to discard:

- Problem / motivation (1-2 lines).
- Scope: new capability, or which existing capability/behavior changes.
- Impact: who/what is affected.
- Draft user stories (one line each).
- Draft high-level acceptance criteria (one line each — not full detail yet).

Reflects the most recent change cycle only; a new cycle overwrites it.

## `spec.md` — Specify + Clarify

specdriven.ai does not give Clarify its own artifact — clarification answers
(edge cases, empty states, error handling) are recorded directly in the
specification. Structure:

```text
Purpose
Capability Overview
User Stories
Business Rules
Edge Cases & Error Handling
Non-Functional Requirements   (only when applicable)
Acceptance Criteria
Access Control
Related Capabilities
Open Questions
```

This is the durable artifact — the capability's living source of truth,
consulted by future spec and implementation work on this capability. It is
not discarded after implementation.

## `design.md` — Plan (conditional)

Only created when the change has real architectural weight: a new state
model, a new cross-layer contract, a new dependency, or a decision spanning
Presentation/Application/Domain/Infrastructure. If none of these apply, skip
it explicitly — do not generate an empty or filler file.

Content: architectural decisions, state/entity models, cross-layer contracts,
dependencies. For Terminal-specific decks, reuse the vocabulary in
`knowledge/architecture.md` (layers, ports, lifecycles).

Reflects the most recent change cycle; updated in place if still relevant to
a later cycle, otherwise overwritten.

## `tasks.md` — Tasks

Atomic tasks (roughly 1-4 hours each), ordered by dependency, each with its
own acceptance criterion. This is the contract `feature-implementation`
implements against.

Reflects the most recent change cycle only; a new cycle overwrites it.

---

# Business Rules

Document behavior in terms of rules.

Good:

> A command cannot start a second block while the shell reports the previous one as Running.

Good:

> Only an exit marker (`OSC 133;D`) transitions a block out of the Running state.

Avoid:

> SessionService calls the PTY adapter directly and blocks the editor during execution.

Implementation details belong to the source code.

---

# User Stories

Required in `spec.md` for the full cycle; capture intent, not mechanics.

Good:

> As a user, I want to copy only the output of a long-running command, so that I can paste logs without retyping the command.

Avoid:

> The system has a copy button.

---

# Acceptance Criteria

Mandatory in `spec.md` whenever the full cycle applies (new capability, or a
behavior change not yet implemented) — this is the contract
`feature-implementation` verifies against. Optional only in
lightweight-documentation mode, for specs that document already-shipped,
stable behavior.

Write each criterion as a testable statement, not a description:

Good:

> Given a command that emits more than 10k lines, the block shows a capped/segmented buffer without unbounded memory growth.

Good:

> Given a `133;D;<code>` marker with nonzero code, the block is marked Completed (Failed) with a red exit badge.

Keep criteria scoped to observable behavior — no algorithm or implementation
detail.

---

# Edge Cases & Error Handling

Required in `spec.md` for the full cycle — specdriven.ai's Clarify content.
Surface underspecified areas explicitly rather than letting them default
silently during implementation:

- Edge cases (empty input, boundary values, concurrency, partial failures).
- Empty/initial states.
- Error handling and the expected error shape/behavior per failure.

For this project, mandatory edge-case families: full-screen app takeover
(alt-screen), infinite output (memory budget), PTY child death mid-command,
and command with no output.

---

# Non-Functional Requirements

Include only when applicable (latency, memory budget, resource-retention
rules explicitly required by the task). The < 80 MB RAM budget is a standing
functional constraint from `knowledge/roadmap.md`, not a speculative NFR. Do not
add other NFRs speculatively.

---

# Ambiguity Handling

Do not make assumptions when behavior cannot be determined.

If an important decision is unclear:

1. Identify the ambiguity.
2. Explain why it affects the specification.
3. Ask the user before creating the final artifact.

Example:

> "The semantic marker `133;D` also fires `Cwd` notifications on some shells. Should the parser treat Cwd updates as a separate event or fold them into the block transition?"

---

# Open Questions

Use Open Questions (in `spec.md`) for unresolved decisions.

Include:

- Security considerations.
- Missing business rules.
- Unexpected existing behavior.
- Decisions requiring confirmation.

Do not hide uncertainty by choosing an interpretation.

---

# Approval Gates

specdriven.ai's critical control point is after Tasks — no code before that
approval. This skill adds two earlier, cheaper gates so a rejection doesn't
waste work already sunk into later artifacts.

Apply all three gates only in the **full SDD cycle**. Skip them entirely in
lightweight-documentation mode, and for small edits to an already-approved
`spec.md` — use Ambiguity Handling instead for anything unclear in that edit.

Deliver each approval question in the user's current language; the wording
below is the canonical English version.

**Gate 1 — Proposal** (before writing `spec.md`)

1. Present `proposal.md`'s content in chat.
2. Ask for approval, in the user's current language: "Do you approve this proposal before I write the full specification?"
3. Wait for explicit confirmation — silence is not approval.

**Gate 2 — Specification** (before writing `design.md` / `tasks.md`)

1. Present `spec.md` (or its outline if long) in chat.
2. Ask for approval, in the user's current language: "Do you approve this specification (user stories, acceptance criteria, edge cases) before moving to design/tasks?"
3. Wait for explicit confirmation.

**Gate 3 — Tasks** (definitive gate; before any implementation)

1. Present `tasks.md` in chat.
2. Ask for approval, in the user's current language: "Do you approve this task plan? No production code will be written until this approval."
3. Wait for explicit confirmation. This holds even if the user's original
   request already implied "just build it" — do not proceed to implementation
   in the same turn without this separate, explicit approval.

If the user requests changes at any gate, adjust that artifact and ask again
before moving to the next one.

---

# Updating Specifications

`spec.md` must not be updated automatically after implementation changes.
Only update it when:

- The user explicitly requests it.
- A documentation task is being performed.
- The business behavior itself is being changed and documentation update is
  part of the approved scope.

`proposal.md`, `design.md` and `tasks.md` reflect only the most recent change
cycle for that capability — a new cycle overwrites them; they are not an
accumulating history.

When updating an existing `spec.md`, use exact-match edits for the changed
sections — do not rewrite the whole file unless most of it changes
(`knowledge/governance/policies.md` § 14).

---

# Closing the Loop — Drift Prevention

After implementation changes behavior that a shipped `spec.md` describes,
files may drift from the spec. Do not edit the spec automatically.

- Point the user at the `drift-audit` skill to verify docs vs implementation.
- If drift is confirmed, updating the spec is a separate explicitly-approved
  documentation task (`knowledge/governance/policies.md` § 10).

---

# Quality Criteria

A good specification should allow someone to understand:

- What the capability does and why (user stories).
- Which rules apply, including edge cases and error handling.
- Who can use it.
- How it relates to other capabilities.

It should not require understanding the source code to understand the
behavior.

---

# Exit Criteria

**Full SDD cycle** is done only if:

- Context was recovered per artifact (see Context Acquisition) — no invented
  business rules.
- Gate 1 was approved before `spec.md` was written.
- Gate 2 was approved before `design.md` / `tasks.md` were written.
- `design.md` was either produced (architecturally significant change) or
  explicitly skipped with a stated reason.
- Gate 3 was approved before any implementation began — non-negotiable.
- `spec.md` structure is complete: Purpose, Capability Overview, User
  Stories, Business Rules, Edge Cases & Error Handling, NFR (if applicable),
  Acceptance Criteria, Access Control, Related Capabilities, Open Questions.
- No ambiguity was silently resolved — unresolved items are in Open Questions.

**Lightweight documentation** is done only if:

- Context was recovered — no invented business rules.
- `spec.md` structure is complete for the relevant sections.
- No ambiguity was silently resolved.

---

# Core Principle

Specifications document **what the system does and why**.

Source code defines **how the system does it**.
