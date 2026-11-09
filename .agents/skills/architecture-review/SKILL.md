---
name: architecture-review
description: Determine which layer or module should own a responsibility, and whether an existing pattern already solves it, without redesigning the system. Use when analyzing where new or changed functionality belongs across the presentation/application/domain/infrastructure layers of the terminal engine, or reviewing whether a proposed change fits existing boundaries. Do not use to implement the change itself — that is feature-implementation.
---

# Architecture Review Skill

## Purpose

Define the workflow agents must follow when analyzing architecture decisions
and recommending where functionality should belong.

The objective is to preserve the existing architecture while making
technically sound recommendations.

# Review Objective

When reviewing architecture, answer:

> Where should this responsibility live and why?

The goal is not to redesign the system.

The goal is to understand existing boundaries and place responsibilities
correctly.

---

# Review Workflow

Before proposing a solution, understand:

- Current architecture (`knowledge/architecture.md`).
- Existing module boundaries.
- Layer responsibilities and the two execution lifecycles.
- Data flow (ports, events, GPUI updates).
- Similar existing implementations.

Review only the required context. Do not analyze the entire repository
without a specific reason.

Once candidate modules/files are identified (via `rg`/`fd`, not full reads),
raw retrieval of "similar existing implementations" may be delegated to a
fast subagent — it fetches content; it does not judge where the
responsibility should live (`knowledge/governance/policies.md` § 13).

Cap: 10 files per turn (`policies.md` § 16); stop and ask before reading
more.

---

# Architecture Analysis

Evaluate the responsibility against the existing Clean Architecture layers:

```text
Presentation (GPUI views, components)
      ↓
Application (services, use cases)
      ↓
Domain (entities, ports, state machines)
      ▲
Infrastructure (PTY, parser, persistence, FS adapters) ── implements Domain ports
```

Determine:

- Which layer owns the responsibility.
- Which module should contain it (`knowledge/project-map.md` entry points).
- Whether existing patterns already solve the problem.

Use the Change Mapping table in `knowledge/architecture.md` as the baseline:
tasks of the same kind have already-known owners; deviations need explicit
justification.

---

# Decision Priority

When multiple solutions exist, prioritize:

## 1. Preserve Existing Architecture

The current project structure has priority.

Avoid introducing new patterns unless the current architecture cannot support
the requirement.

---

## 2. Maintainable Solution

Prefer solutions that:

- Keep responsibilities separated.
- Reduce coupling.
- Improve future maintainability.

---

## 3. Fastest Acceptable Solution

When several solutions are valid, prefer the simpler implementation.

Avoid unnecessary complexity.

---

# Recommendation Format

Architecture recommendations should explain:

1. Proposed location.
2. Responsibility ownership.
3. Reasoning.
4. Potential impact.

Example:

```text
The file watching integration should live in infrastructure (src/infrastructure/filesystem/)
because it is an OS dependency. The application layer should consume it
through the FileSystemPort so the Domain model stays pure and the Tools
Panel reacts to Domain events, not to notify's raw types.
```

---

# Code Changes

This skill is for analysis and recommendations.

Do not modify code unless the user explicitly requests implementation.

---

# Avoid

Do not recommend:

- New architectural patterns only because they are popular.
- Technology changes without a requirement (new crates need the low-memory
  approval in `AGENTS.md` § STOP).
- Large restructures without approval.
- Moving code without understanding current responsibilities.

---

# Core Principle

The best architecture decision is the one that fits the existing system while
keeping responsibilities clear.
