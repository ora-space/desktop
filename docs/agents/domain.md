# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

This repo uses the **multi-context** layout: a root `CONTEXT-MAP.md` points to one `CONTEXT.md` per context (e.g. per package/crate/app as they get created). System-wide decisions live in `docs/adr/`; context-scoped decisions live under that context's `docs/adr/`.

## Before exploring, read these

- **`CONTEXT-MAP.md`** at the repo root: it points at one `CONTEXT.md` per context. Read each one relevant to the topic.
- **`CONTEXT.md`** for the specific context you're working in.
- **`docs/adr/`**: read ADRs that touch the area you're about to work in. In multi-context repos, also check the context's own `docs/adr/` for context-scoped decisions.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved.

## File structure

Multi-context repo (presence of `CONTEXT-MAP.md` at the root — this repo's layout):

```
/
├── CONTEXT-MAP.md
├── docs/adr/                          ← system-wide decisions
├── packages/
│   ├── chat/
│   │   ├── CONTEXT.md
│   │   └── docs/adr/                  ← context-specific decisions
│   └── editor/
│       ├── CONTEXT.md
│       └── docs/adr/
└── crates/
    └── core/
        ├── CONTEXT.md
        └── docs/adr/
```

Single-context repo (most repos), for reference:

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-event-sourced-orders.md
│   └── 0002-postgres-for-write-model.md
└── src/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in the relevant `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
