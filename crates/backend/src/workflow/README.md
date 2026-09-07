# Workflow Backend Module

This module composes the backend-facing workflow definition API and the runtime adapters required
to execute workflow runs. It is the workflow capability boundary inside `ora-backend`; transport
commands remain in their adapters and graph scheduling remains in `ora-application`.

## Responsibilities

- `definition.rs` wires workflow definition, draft, publication, and version handlers to SQLite.
  Its public `WorkflowApi` is reached through `Backend::workflows()`; construction and concrete
  repositories stay private. It projects application errors before returning to adapters and
  can be tested against SQLite without assembling any workflow execution runtime.
- `run/` wires workflow-run CRUD, the production execution engine, agent-node execution, prompt
  assembly, deployment prerequisites, and interactive-node coordination.

## Boundaries

The module depends on application ports and concrete backend infrastructure, but it does not own
workflow domain entities, transport contracts, persistence implementations, or DAG scheduling.
Those responsibilities remain in `ora-domain`, `ora-contracts`, `ora-db`, and `ora-application`.
