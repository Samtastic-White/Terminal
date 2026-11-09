# Project Map

## Purpose

**Question answered by this document:**

> Where should an agent start, what should it read, and how should it navigate the repository?

This document defines the official navigation strategy of the project. Its
objective is to minimize unnecessary source code exploration by providing a
predictable path from request intent to implementation. Source code is always
the final source of truth.

---

# First Navigation Decision

Before reading any source code:

```text
Request received
        │
        ▼
Is there a related capability specification?
        │
   ┌────┴─────┐
   │          │
  Yes         No
   │          │
   ▼          ▼
Read             Did the user provide
knowledge/specs/           a file path?
<capability>/spec.md     │          │
first                   Yes         No
                         │          │
                         ▼          ▼
                    Read that   Follow AGENTS.md §
                    file only   Context Acquisition Order
                                (roadmap.md → architecture.md
                                 → domain → implementation)
```

Do not skip the specification when one exists. Do not read implementation
files before the intent is confirmed.

---

# Question → Document Routing

| Question | Document |
| --- | --- |
| What does the product do, why? What remains to build? | `README.md` (canonical) + `README.es.md` (Spanish translation) + `knowledge/roadmap.md` |
| Which layer owns this change? What is the Change Mapping? | `knowledge/architecture.md` |
| How do I write/format/test code here? | `knowledge/conventions.md` |
| What requires approval, what is forbidden? | `knowledge/governance/policies.md` |
| Why do these rules exist? | `knowledge/governance/constitution.md` |
| Why X and not Y (past decisions)? | `knowledge/governance/decisions/` |
| Hard rules that load every session? | `AGENTS.md` (already in context) |

`README.es.md` is a Spanish translation of `README.md`; if they disagree, the
English canonical file wins. Every other documentation file is English-only.

---

# Task Type → First File

For layer ownership and Change Mapping, use the table in
`knowledge/architecture.md` § Change Mapping — it is not repeated here.
Concrete entry points per layer:

```text
src/
├── domain/
│   ├── block.rs                  # Block entity + BlockStatus lifecycle
│   └── semantic.rs               # SemanticEvent variants (OSC 133 markers)
│   + Port traits (PtyPort, HistoryRepository, FileSystemPort)
├── application/
│   ├── session_service.rs        # shell lifecycle, execute_command
│   ├── block_service.rs          # block state updates, sticky headers
│   └── history_service.rs        # history queries
├── infrastructure/
│   ├── pty/                      # portable-pty adapter
│   ├── parser/                   # ANSI lexer + OSC 133 → SemanticEvent
│   ├── persistence/              # in_memory.rs, sqlite.rs (Phase 5+)
│   └── filesystem/               # Tools Panel directory watching
├── presentation/
│   ├── window.rs                 # bootstrap, layout, split panes
│   ├── components/
│   │   ├── block_card.rs         # card container, badges, copy buttons
│   │   ├── sticky_header.rs      # pinned command header
│   │   └── input_editor.rs       # multi-line GUI input (ZLE substitute)
│   └── theme/                    # design tokens
└── common/
    ├── errors.rs                 # Result<T, TerminalError>
    └── id.rs                     # block ID generation
```

Canonical task flows:

- **New OSC semantic code:** `src/infrastructure/parser/` → variant in `src/domain/semantic.rs` → handler in `src/application/`.
- **Command execution behavior:** `src/application/session_service.rs` + `src/domain/block.rs`.
- **History persistence (Phase 5+):** implement `HistoryRepository` in `src/infrastructure/persistence/`.
- **Visual component:** `src/presentation/components/` — never touch Domain.

---

# Discovery Hierarchy

1. The request itself (file paths, quoted symbols).
2. Spec-driven documents (`knowledge/specs/<capability>/spec.md`).
3. `knowledge/architecture.md` (Change Mapping, entry points above).
4. Symbol search: `rg <symbol> src/`, `fd <name> src/` — a hit is a hypothesis,
   not a reason to read.
5. Targeted file reads — only files the hypothesis names.
6. Subagent delegation for mechanical sweeps — with the caps in
   `knowledge/governance/policies.md` § 13 and § 16.

Hard boundary: do not read file contents until a symbol-level search has
narrowed candidates to specific files. A search hit names candidates; it does
not authorize bulk reading.

---

# Exploration Budget

- Never list the repository unbounded (`ls -R`, full `tree`); use
  `fd --max-depth=N` or `fd --type d` for structure questions.
- Cap: 10 file reads per analysis turn (`knowledge/governance/policies.md` § 16).
  At the cap: stop, summarize, ask before continuing.
- Stop rule: if exploration touches more than 5 unrelated files without
  answering the question, summarize what is known and ask (`AGENTS.md` § 7).

---

# Local Reference Implementations (Read-Only)

Sibling repositories in the parent directory provide proven
implementations to consult during analysis. Check these locally (via
`rg`/`fd`) before fetching anything from the network. They are never a
source of truth for this repository, and code is never copied into
`src/` without an explicit licensing decision
(`knowledge/governance/policies.md`).

| Repository | What it is | Consult for |
| --- | --- | --- |
| `../zed-gpui/` | The `gpui` crate only (Apache-2.0), pruned Zed clone | GPUI API ground truth: `crates/gpui/src/elements/list.rs` (`ListState`, height hints, `FollowMode::Tail`), `uniform_list.rs`, `div.rs`. It does NOT contain Zed's `terminal` crate |
| `../warp-terminal/` | Warp workspace in Rust on `vte` (AGPL-3.0 majority + MIT parts; alacritty-derived files carry `LICENSE-ALACRITTY`). Read-only, visual/UX-first reference — do not copy | Phase 1 writer: `crates/warp_terminal/src/model/ansi/` + `model/grid/` + `blockgrid.rs`; Phase 2: `escape_sequences.rs` + `ESCAPE_SEQUENCES.md`; Phase 4: `mode.rs` + `mouse.rs` (DECSET bits, mouse encodings) |
| `../wave-terminal/` | Wave Terminal (Apache-2.0; Electron + Go) | Product-level UX only: `frontend/app/block/`, `pkg/blockcontroller` — its stack violates the memory budget and is never an implementation dependency |

Quick phase mapping: F1 block writer → warp `model/ansi` + `model/grid`;
F2 OSC 133 → warp `escape_sequences`; F3 editor → wave `frontend/app/block/`
UX; F4 mouse protocol → warp `mouse.rs`; F6 virtualization → zed-gpui
`elements/list.rs`.

---

# Validation Short-Circuits

- **Trust the compiler:** when unsure whether a type/trait/method exists or the
  call shape is right, run `cargo check` in the container instead of guessing
  (`AGENTS.md` § 7).
- **Semantic markers:** OSC 133 → parser lives in `src/infrastructure/parser/`;
  event model lives in `src/domain/semantic.rs`. UI never parses escape bytes.
- **Verification is not optional:** `cargo check && cargo clippy -- -D
  warnings && cargo test` before any "done" report (`AGENTS.md` § 8).
