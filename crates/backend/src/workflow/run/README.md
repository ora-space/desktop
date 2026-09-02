# Workflow Run Backend Module

This module adapts workflow-run application use cases to the production backend environment.

## Responsibilities

- `api.rs` composes workflow-run CRUD handlers and worktree provisioning.
- `engine.rs` builds the production run engine, attaches callbacks, and resumes recoverable runs.
- `executor.rs` drives agent nodes through Ora sessions, publishes each session only after its
  owning prompt is admitted, and records node outputs and file changes.
- `prerequisites.rs` resolves roles and freezes required-skill paths through an injected Agent
  delivery-capability provider. Effect owns physical Skill materialization. The current provider
  declares the shared `.agents/skills` root; future plugin-backed providers can declare different
  or multiple roots without changing the workflow or prompt layers.
- `prompt.rs` assembles the localized, worktree-bounded, topology-aware handoff for an agent node.
  Required-skill constraints show the actual absolute package paths resolved from the frozen run
  receipt while preserving leading slash commands for Agent CLI parsing. All Agent-facing paths
  use forward-slash separators consistently, independent of the host operating system. Text block
  boundaries include explicit blank lines because Agent providers may concatenate ACP blocks
  without adding separators.
- `interactive/` coordinates human turns and manual completion for interactive nodes.

## Boundaries

DAG parsing, scheduling, and durable node-run transitions are owned by `ora-application`. This
module supplies concrete execution and infrastructure adapters without duplicating that state
machine.
