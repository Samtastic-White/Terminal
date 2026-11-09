# Decision Records (ADRs)

One decision per file, created as decisions happen. These archive **why** this
project chose X over Y, so future agents don't re-litigate settled choices.

## Convention

- File name: `NNNN-kebab-case-title.md` (`0001-gpui-over-ttk.md`);
  `NNNN` is zero-padded and strictly increasing.
- Content, five short sections:

```markdown
# NNNN. <Title>

## Status
accepted | superseded by NNNN

## Context
The forces and constraints that made this decision relevant (2-5 bullets).

## Decision
What was decided, in one or two imperative sentences.

## Consequences
What becomes easier, what becomes harder, what is now forbidden.

## Alternatives considered
The runner-ups and why they lost.
```

- Never delete an ADR. Supersede it via the Status field pointing to its
  replacement.
- Related policy text (must-do rules) belongs in `policies.md`, not here.
  This folder is history, not law.
