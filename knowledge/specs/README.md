# Capability Specifications

Specifications produced by the Spec-Driven Development flow
(`.agents/skills/create-specification/SKILL.md`): capture intent before code
exists, gate explicitly before implementation, keep the specification as the
capability's living source of truth afterward.

## Convention

- One directory per capability: `specs/<capability>/`.
- Full SDD cycle produces, in order and each gated before the next:
  `proposal.md` → `spec.md` → `design.md` (conditional) → `tasks.md`.
- Lightweight documentation of already-shipped behavior writes/updates
  `spec.md` only.
- No production code before the Tasks gate is approved.

Directories are created by the skill itself when the first capability starts.
Never place implementation notes here — source code remains the
implementation source of truth for how things work.
