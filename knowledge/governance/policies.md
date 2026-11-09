# Policies

Defines what agents may do autonomously, what requires approval, and what's
forbidden. Not architecture, conventions, or business rules. A policy takes
priority over implementation convenience.

## Categories

- **Allowed** — the agent may perform the action autonomously.
- **Requires Approval** — the agent must explain the reason, explain the
  impact, and wait for explicit approval.
- **Forbidden** — the agent must never perform the action.

## 1. Secrets and credentials

Forbidden:

- read, write, create, or delete `.env` files or any environment file variant;
- access secret material — private keys, certificates, credentials,
  authentication secrets — unless explicitly required and approved by the user.

Enforcement: `.env*` read/edit is hard-blocked via `permissions.deny` in
`.claude/settings.json` (deny includes read/edit for Claude Code) and
`.opencode/opencode.jsonc`. Other secret material is not hard-denied there —
that rule allows an explicit-approval exception a `deny` rule can't express,
so it remains governed by this text.

## 2. Scope

Allowed: modify files directly required to complete the requested task.

The agent must not expand beyond the requested scope without approval.
Requires approval: unrelated refactors, cleanup, performance improvements not
requested, architectural improvements, changing existing behavior outside the
task, optimizing unrelated code, modernizing dependencies without request.

See `AGENTS.md` § STOP general clause for the explain → justify → wait
procedure.

## 3. Existing code

Allowed: create files the task needs; modify existing implementation; remove
code created during the current task if it becomes unnecessary.

Requires approval: removing existing files, functions, traits, or modules.

Finding unused code does not authorize removal — report it, don't delete it.

## 4. Refactoring

Allowed: small local refactors required for implementation — extracting a
helper, improving naming, reducing duplication inside the affected area.

Boundary rule: a refactor counts as "required for implementation" only if it
touches lines the task would already modify. A refactor that reaches into code
the task would not otherwise touch is discretionary — even if small — and
falls under the `AGENTS.md` § STOP general clause (explain → justify → wait).

Requires approval: large refactors — moving functionality between layers,
reorganizing architecture, renaming modules, restructuring folders, rewriting
existing systems.

## 5. Dependencies

Requires approval: before adding a dependency, explain the purpose, explain
alternatives if relevant, receive approval. Stack and package manager:
`AGENTS.md` § Stack.

Forbidden: installing or removing dependencies automatically; replacing the
build system; searching for removable dependencies unless explicitly requested.

Mandatory after approval, before reporting the task done: install with the
exact-version pin (e.g. `cargo add serde@1.0.203`, or edit `Cargo.toml`
directly with the exact version), then re-read the touched `Cargo.toml` lines
and confirm the exact approved version is recorded. Every crate must pass the
low-memory filter (< 80 MB RAM budget) stated in `AGENTS.md` § STOP.

## 6. Commands and execution

Allowed: read-only commands for inspecting files, checking structure,
validating assumptions (listing directories, searching files, reading source
code).

Requires approval: commands that install packages, modify dependencies,
generate files, modify project configuration, affect external services, or
perform destructive operations (e.g. database resets, destructive migrations).

## 7. Testing and validation

Allowed: running existing validation commands already available (`cargo
check`, `cargo test`, `cargo clippy`, `cargo fmt --check`).

Requires approval: creating new test infrastructure, changing test
configuration, adding testing dependencies, modifying CI/CD configuration.

## 8. Persistence

Allowed: modifying SQLite queries, schemas, or scripts required by the
requested functionality (Phase 5+, via `src/infrastructure/persistence/`).

Requires approval: changes unrelated to the requested task — persistence
redesign, unrelated cleanup or optimization, restructuring tables without
requirement.

## 9. Git operations

Forbidden: any git operation (add, commit, push, merge, rebase, checkout,
reset, etc.) — always user-controlled.

Enforcement: mutating `git *` commands are hard-blocked via
`permissions.deny` in `.claude/settings.json`; read-only git inspection
(`status`, `diff`, `log`, `show`) remains allowed in `.opencode/opencode.jsonc`
and denied-by-ask elsewhere.

## 10. Documentation

The agent must not automatically update documentation — README files,
`AGENTS.md`, `knowledge/` documents, or `knowledge/specs/**`. This holds even
when the change is a direct, mechanical consequence of an approved edit (e.g.
renaming a field that's also referenced in a spec). Consequence does not imply
consent: surface the inconsistency and ask before editing — never edit first
and explain afterward.

Exception: if the user explicitly requests documentation as part of the task,
the agent may update the requested documentation.

## 11. Agent configuration

Requires approval: modifying `AGENTS.md`, `CLAUDE.md`, `knowledge/`,
`knowledge/governance/`, `knowledge/specs/`, `.agents/skills/`, `.claude/`,
`.opencode/`. The agent may recommend improvements without applying them.

Enforcement: editing/writing any of these paths is gated via `permissions.ask`
in `.claude/settings.json` and `permissions` in `.opencode/opencode.jsonc`.

## 12. Security

Forbidden: intentionally accessing confidential information outside the task
scope.

Requires approval: when identifying a security issue — 1. explain the
vulnerability, 2. explain the possible impact, 3. propose a solution, 4. wait
for approval before implementing.

## 13. Subagent delegation for mechanical tasks

Mandatory (not merely allowed), on any engine that supports subagents: for
deterministic, judgment-free work — system/installation diagnostics
(live-environment inspection such as `docker ps`/`docker logs`/`docker exec`,
read-only SQL queries against a running database), mass file discovery (`fd`,
`rg`, directory listings), and lint/build/test verification loops
(`cargo check`/`clippy`/`test`/`fmt`) — delegate to a dedicated fast subagent
(e.g. `model: "haiku"` under Claude Code, or the cheapest capable subagent
available) instead of running it on the main thread. "I need to interpret the
result anyway" is not a valid reason to skip delegation — that judgment call
happens only after the subagent reports a failure; it does not license running
the check itself on the main thread.

This also covers the initial discovery / code-analysis phase of any task that
requires scanning multiple files to figure out what's relevant — feature
scoping, migration analysis, "analyze the repo before X" requests,
architecture-review sweeps. The main thread must not perform the scanning pass
itself; it reads only the specific files the subagent has already identified,
and only after that filtered list exists. See § 16 for the hard limits
governing that pass.

Rationale: this class of work requires no domain judgment. Running it on the
main thread consumes the most valuable context window (and, on Claude-class
setups, mid-session model switches can invalidate the main thread's prompt
cache; a subagent isolates the switch instead). Delegating mechanical passes
is a pure context/token reduction with no quality tradeoff on this class of
task.

Boundary: delegate only the mechanical step itself (running the command,
reading raw output, reporting pass/fail or a condensed status back).
Interpreting failures, deciding what to fix or check next, and anything
requiring business or architectural judgment stays on the main thread — but
that judgment is applied to the subagent's condensed report, not by reading
the raw log/table/build output directly on the main thread. Do not delegate
specification/design writing or code implementation — see the Discovery
Hierarchy in `constitution.md` for what still requires the main thread's
judgment.

Root cause this section closes (2026-08-14): during a live integration
verification, repeated build/test confirmation runs and an entire
live-Docker debugging session (container status, logs, SQL queries, queue
inspection) ran directly on the main thread instead of being delegated,
driving context consumption far above what the task required — justified in
the moment as "needing to interpret the result," which conflated the judgment
step (only needed on failure) with the mechanical execution step (never needs
the main thread).

## 14. Output and editing efficiency

Forbidden: using a full-file rewrite to replace an existing file longer than
~30 lines when the change touches less than roughly 20% of it.

Allowed: full rewrites for new files, files at/under ~30 lines, or changes
touching most of the file.

Mandatory default: for any change to an existing file, use the exact-match
edit tool (patch/replace) instead of rewriting the whole file — it sends only
the changed text. Do not substitute `sed`/`awk` for the structured edit tool.

Output display: after a subagent or tool call produces a full file, do not
re-print the whole file's content in the response. Show only the changed
lines (diff) or the command's result — the tool call already carries the full
content; repeating it in prose is redundant.

Rationale: main-thread output tokens are the most expensive cost in this
workflow — reducing them is a direct cost/latency win with no quality
tradeoff, mirroring § 13's logic on the input side.

Boundary: governs how a change is applied/displayed, not whether it should be
made — § 2 (Scope) and other categories still apply independently.

Enforcement: behavioral only — not expressible via permission rules
(file-size/diff-ratio isn't something the permission system evaluates).

## 15. Subagent correction discipline

Mandatory before delegating: the plan handed to the subagent must be precise
enough that syntax/API errors are unlikely — concrete file paths, verified
imports/signatures (read the actual source, not assumed from memory), and
exact patterns from the relevant skill when one exists.

Forbidden: entering an explanation loop with the subagent — diagnosing a
conceptual/logic error and sending a second explanatory correction after a
first fix attempt hasn't fully resolved the issue.

Allowed: exactly one (1) automatic correction round, only for a trivial,
single-fact error (deprecated/wrong API name, typo, wrong import path, narrow
pattern fix). The message must be a directive substitution ("replace X with
Y"), not an explanation of underlying semantics.

Requires hand-off: if the error is not trivial, or persists after the single
correction round — stop delegating immediately. Do not send a third round
under any framing. Either ask the user how to proceed, or implement the fix
directly on the main thread, bypassing the subagent for the rest of that
change.

Rationale: correcting a smaller model's conceptual misunderstanding costs
main-thread output tokens at a much higher rate than delegating saves. Past
one correction, fixing it directly is cheaper than continuing to "teach" the
subagent. Complements § 13 (input-side delegation) and § 14 (output economy).

Boundary: governs the delegation loop, not whether delegation was appropriate
to begin with (§ 13 covers that).

Enforcement: behavioral only.

## 16. Exploration budget — subagents and file reads

Root cause this section closes: a fork launched for a single "analyze before
X" request spawned four nested exploration subagents (fan-out depth 2), which
together read 35 files and produced ~85k output tokens before any
implementation started — all unfiltered.

Forbidden:

- reading file contents during the initial exploration phase of a task,
  before name/symbol-level search (`glob`, `grep`, `rg`, `fd`) has narrowed
  candidates to specific files;
- launching a fork or subagent for search/analysis around a broad,
  undecomposed goal ("analyze the repo", "review the whole PTY flow");
  decompose into concrete, narrow questions first — each agent gets one
  question, not a domain;
- a subagent spawning further exploration subagents (nested fan-out) to cover
  a discovery pass that a single targeted agent, or the mechanical pass in
  § 13, can answer; nested spawns start cold contexts and cannot share the
  parent's cache, so fan-out multiplies cost instead of parallelizing useful
  work;
- `ls -R`, `tree`, or an unbounded directory dump as a discovery step; use
  depth-limited listing (`fd --max-depth=N`) or `fd --type d` instead.

Mandatory limits on any subagent (fast model or otherwise) doing
discovery/analysis:

- maximum 10 file reads within a single analysis turn;
- on reaching that limit, the subagent must stop, summarize what it has
  learned so far, and explicitly request permission before reading further
  files — it must not silently continue past the cap.

Allowed: a subagent may read more than 10 files across a task's full lifetime,
but only in successive turns the user (or the orchestrating thread) has
explicitly permitted after reviewing the interim summary — not in one
uninterrupted turn.

Rationale: complements § 13 (which agent does the scanning) with a hard cap on
how much scanning happens before a checkpoint — the two failure modes (wrong
executor, unbounded scope) compound independently and both must be closed.
Also complements `constitution.md` § Context economy, which states the same
discipline in reasoning terms.

Enforcement: behavioral only — file-read counts and fan-out depth aren't
expressible via permission rules.

## Final policy

If an action may affect code, files, architecture, dependencies,
configuration, or project behavior outside the explicit scope: stop, explain
why the action is necessary, request approval, continue only after approval.

Agent autonomy exists to complete tasks correctly, not to expand them.
