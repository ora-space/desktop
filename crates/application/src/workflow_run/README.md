# Workflow Run Application Module

This module implements transport-independent CRUD use cases for workflow runs: each run freezes one published snapshot, executes inside a caller-selected Workspace, stores its own display name, and can be created, listed, read, renamed, and soft-deleted.

## Responsibilities and boundaries

- Creation resolves the frozen snapshot (an explicit `snapshot_id` or the workflow's `published_snapshot_id`), validates it is published and belongs to the requested workflow, admits the selected active Workspace, resolves enabled skills against each Agent's delivery capability, and atomically persists the Workspace-owned run. Effect owns physical Skill materialization; workflow creation only freezes the display locale together with each node's invocation name and workspace-relative package paths. Runs start `Pending` with an empty `current_nodes` anchor; an omitted kickoff input inherits the frozen Start instruction, while an explicit value takes precedence. A missing run name receives a deterministic default from the workflow name and creation time.
- Get and list operations expose visible records only. Detail queries return the run's own display name, direct Workspace identity, project projection, and node-run history; list queries return lightweight summaries scoped to one project.
- Node-run history is read-only here — node-run creation, updates, and their state machine belong to the execution engine, which writes through the same repository.
- Soft deletion refuses active runs (a `Running` run, a non-terminal node run, or a `Running` session bound to one of the run's node runs) and then cascades a soft-delete across the run, its node runs, and those node-owned sessions in one transaction. A not-started `Pending` run is not active. It never deletes or retires the shared Workspace.
- Snapshot protection lives in the `workflow` module: a published snapshot referenced by a live run cannot be soft-deleted, and a workflow whose snapshots a live run freezes cannot be deleted either, so a run's frozen graph stays readable across its lifecycle.

`WorkspaceRepository`, `WorkflowRunRepository`, `WorkflowRunIdGenerator`, `WorkflowRunWorkspaceInitializer`, and `Clock` isolate storage, identity, workspace preparation, and time from the handlers. Creation parses the frozen graph to validate deployment prerequisites, capture the materialization receipt, and resolve the default kickoff input, but this module does not execute workflows or choose transport semantics — those belong to the [execution engine](engine/README.md) and transport adapters. `ora-backend` implements `NodeExecutor` as `WorkflowRunNodeExecutor` and wires it in `Backend::open`.

See the [ora-application overview](../../README.md) and [Workflow architecture doc](../../../../docs/workflow.md).
