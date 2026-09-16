# Workflow Run Backend Module

This module adapts workflow-run application use cases to the production backend environment.

## Responsibilities

- `api.rs` composes workflow-run CRUD handlers and worktree provisioning.
- `engine.rs` builds the production run engine, attaches callbacks, and resumes recoverable runs.
- `executor.rs` drives agent nodes through Ora sessions, publishes each session only after its
  owning prompt is admitted, freezes the node's enabled MCP bindings as the Session's explicit
  permission set, and records node outputs and file changes.
- `prerequisites.rs` resolves roles and freezes required-skill paths through an injected Agent
  delivery-capability provider. Effect owns physical Skill materialization. Skill resolution is
  origin-aware: a local skill resolves through its formal catalog directory and a plugin-imported
  skill through the immutable plugin package recorded in its catalog row. The current provider
  declares the shared `.agents/skills` root; future plugin-backed providers can declare different
  or multiple roots without changing the workflow or prompt layers.
- `prompt.rs` assembles the localized, worktree-bounded, topology-aware handoff for an agent node.
  Required-skill constraints show the actual absolute package paths resolved from the frozen run
  receipt while preserving leading slash commands for Agent CLI parsing. All Agent-facing paths
  use forward-slash separators consistently, independent of the host operating system. Text block
  boundaries include explicit blank lines because Agent providers may concatenate ACP blocks
  without adding separators.
- `interactive/` coordinates human turns and manual completion for interactive nodes.
- `recovery.rs` runs the graph-aware boot sweep: interrupted rows inside a still-`Running`
  iteration region fail as `interrupted_by_restart` while the composite row and its run survive,
  so the runtime settles the interrupted round as a failed ledger entry on the next advance
  (ADR "iteration composite runtime" D2); anything else keeps the pre-composite whole-run
  failure. `iteration_tests.rs` holds the end-to-end iteration verification against real SQLite.
- `iteration_tests.rs` verifies the composite runtime end to end: serial foreach, the empty
  source, the startup safety ceiling, both error strategies, per-round Condition decisions, the
  outer `current_nodes` anchor, and restart resets.
- `transitions.rs` commits the node-run transitions that happen outside the scheduling engine —
  an interactive node parking at awaiting input, a human turn beginning, and a turn ending —
  through one sink that publishes the run invalidation only when the guarded transition commits,
  sharing the engine's invalidation mechanism (ADR "node runtime orchestration" D7).

## Boundaries

DAG parsing, scheduling, and durable node-run transitions are owned by `ora-application`. This
module supplies concrete execution and infrastructure adapters without duplicating that state
machine. Session MCP recovery reads the selection persisted on the Session itself; this module
does not reconstruct permissions from node-run relationships.
