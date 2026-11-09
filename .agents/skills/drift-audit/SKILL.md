---
name: drift-audit
description: Audit whether AGENTS.md, knowledge/ documents and knowledge/specs/**/spec.md still match the real implementation in src/ — detect dead references (docs pointing at files/functions/routes that no longer exist), desync (code behaves differently than documented), and orphaned code (implemented but undocumented). Use when the user asks to verify docs/specs are still accurate, or after a refactor/rename/removal to see what documentation now lies. Read-only: do not fix findings automatically.
---

# Drift Audit Skill

## Purpose

Detect drift between the documentation set (`AGENTS.md`, `knowledge/**.md`,
`knowledge/governance/**.md`, `knowledge/specs/**/spec.md`) and the real
implementation in `src/`: file, function, type, and module references that no
longer exist, and places where the code's actual behavior contradicts what
the docs claim.

This is a **read-only investigation**. No code or documentation file is
edited as part of this skill — see "Findings vs. Fixes" below.

## When to use

- User asks to check whether docs/specs are still accurate ("is the documentation still accurate?").
- After a non-trivial refactor, rename, or module removal, to see what documentation now lies.
- Periodic governance/audit requests.

Do not use to implement fixes.

## Workflow

### 1. Classify and scope

This is always an investigation per `AGENTS.md` § Task Classification. Default
scope is every markdown file under `knowledge/`, plus `AGENTS.md` itself,
against all of `src/`. If the user named a narrower scope (one spec, one
knowledge file), respect it — don't silently widen it.

### 2. Map references (main thread; no subagent needed for this step)

For each doc file in scope, extract every concrete, checkable claim. Two
distinct claim families exist — both are mandatory, not optional:

**A. Existence/behavior claims** (typically from `knowledge/specs/**/spec.md`,
`knowledge/architecture.md`, `knowledge/project-map.md`):

- File/module paths (e.g. `application/block_service.rs`)
- Type/trait/method names and signatures, including names mentioned only in
  prose (an event called `DirectoryChanged` in a sentence is a claim even if
  no code block spells it out — it must be checked, and a
  same-behavior/different-name match is **Desynchronization**, not "confirmed").
- Behavioral rules stated as fact ("transitions to `Completed` when X",
  "buffers capped at Y", "`precmd` emits `133;D` before `133;P;Cwd=`").
- Protocol claims: OSC 133 marker meaning/grammar in `AGENTS.md` § 10,
  `knowledge/roadmap.md` (OSC 133 Semantic Protocol).

**B. Style/convention rules** (from `knowledge/conventions.md`, but also the
invariant sections of `AGENTS.md` and `knowledge/architecture.md`) — absolute
statements ("never...", "always...", "zero...", numeric limits, naming
patterns, layer-crossing bans):

- Naming conventions (file/type naming patterns).
- Import boundaries (e.g. "domain never imports gpui/portable_pty/rusqlite").
- Allocation/buffer bans (e.g. "no unbounded String accumulation for output").
- Size limits or profile settings stated as project policy.
- Verification policy ("zero warnings" — is `clippy -- -D warnings` actually
  clean?).

Convention rules are not existence checks — they require actively searching
the codebase for violations, not just confirming a referenced symbol exists.
Do not treat conventions as lower-priority than the specs; give them the same
claim-extraction rigor and their own dedicated subagent(s) in step 4.

Enumerate the doc set with
`fd -e md . knowledge | sort` before reading anything.

### 3. Map the reverse direction — code → doc (orphan detection)

Step 2 only checks "doc says X, does X exist in code." It cannot find code
that exists without any doc mention — that requires enumerating the code's
own surface first, independent of any doc claim:

- List layer modules: `fd -t d . src/ -d 2` and `fd -e rs . src/ -d 2`.
- List public ports/services: `rg -n "pub trait|pub struct|pub enum" src/domain src/application`.

For each module/type found this way, search the doc set
(`rg -l "<module-or-type-name>" knowledge AGENTS.md`) for at least one
mention. Zero matches is an **Orphaned Code** candidate — hand it to a
subagent in step 4 to confirm it's genuinely undocumented (not just named
differently, which would instead be a step-2 naming/desync case).

This step runs once per audit, on the main thread — it's a single enumeration
pass, not per-doc-file work.

### 4. Mechanical validation — delegate, don't read everything yourself

Per `knowledge/governance/policies.md` § 13, delegate the multi-file
discovery/validation pass to fast subagents — one per doc file or small doc
cluster, launched in parallel in a single message. Include the orphan
candidates from step 3 as one additional cluster, assigned to a subagent
whose only job is to confirm each candidate has zero doc mentions (not just a
naming mismatch).

Each subagent receives:

- The exact list of claims extracted in step 2 for its assigned doc file(s),
  spelled out — not "go audit this file," which forces it to re-derive scope.
  (The orphan-candidate subagent instead receives the module/type list from
  step 3 with zero doc matches.)
- For **existence/behavior claims**: use `fd` to confirm the referenced
  file/symbol exists, then `rg` for the exact name to confirm the code still
  uses it (a renamed/aliased match is `desynced`, not `confirmed`).
- For **style/convention claims**: use `rg` to actively search for
  violations — e.g. `rg "gpui|portable_pty|rusqlite" src/domain/` for the
  domain-isolation ban, `rg "\.unwrap\(\)" src/ --glob '!*tests*'` for the
  unwrap ban. Absence of a violation is `confirmed`; any match is `desynced`
  with every occurrence listed, not just the first.
- The `policies.md` § 16 cap: at most 10 files read per agent per turn, and
  no spawning of further exploration agents — if it needs more, it stops and
  reports partial results.
- A required output shape: per claim, one of `confirmed | dead | desynced`,
  plus one line of evidence (`file:line`, or the full list of matches for a
  convention violation).

**Coverage rule:** every doc file in scope gets at least one dedicated
subagent and its own claim list — never fold `conventions.md`'s rules into a
generic "style" afterthought handled by whichever agent has spare capacity,
and never skip a doc file because its claims don't fit the file/type
template.

### 5. Synthesize findings

Consolidate every subagent's report into one final table, grouped as:

| Category | Meaning |
|---|---|
| **Dead References** | Doc mentions a file/type/behavior that no longer exists. |
| **Desynchronization** | Code exists but behaves differently than documented (wrong name, wrong transition, wrong protocol order). |
| **Orphaned Code** | Code implements a capability with no corresponding doc/spec at all. |
| **No Drift** | Confirmed match — state these explicitly so the audit reads as complete coverage, not just a problem list. |

### 6. Findings vs. fixes

Present the table and stop. Do not edit any file — doc or code — as part of
this skill, even when a fix looks mechanical. Acting on a finding (updating a
spec, deleting dead code, fixing a convention violation) is a separate task
that follows `AGENTS.md`'s normal classification and STOP rules: ask the user
which findings they want acted on, and how, before touching anything.

## Avoid

- Reading full files when `fd`/`rg` can confirm or deny a claim faster.
- Launching a broad "audit everything" subagent instead of the scoped
  per-doc agents described above.
- Editing `knowledge/`, `AGENTS.md`, or code as a side effect of "obviously"
  fixing a finding.

## Core Principle

The audit's value is in surfacing where documentation lies, not in silently
fixing it. Report completely — dead references, desync, orphaned code, and
confirmed matches alike — then stop and let the user decide what happens to
each finding.
