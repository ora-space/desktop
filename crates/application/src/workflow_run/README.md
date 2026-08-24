# Workflow Run Application Module

This module implements transport-independent CRUD use cases for workflow runs: each run freezes one published snapshot, owns a dedicated run-task and Git worktree, and can be created, listed, read, and soft-deleted.

## Responsibilities and boundaries

- Creation resolves the frozen snapshot (an explicit `snapshot_id` or the workflow's `published_snapshot_id`), validates it is published and belongs to the requested workflow, provisions a dedicated Git worktree, validates every declared role, materializes enabled skills according to each Agent's delivery capability, and atomically persists the worktree, run-task, and run in one repository-managed transaction. The run payload freezes the display locale together with each node's actual skill invocation name and worktree-relative package paths. Runs start `Pending` with an empty `current_nodes` anchor; an omitted kickoff input inherits the frozen Start instruction, while an explicit value takes precedence. The default run-task title is `"{workflow.name} {创建时间}"`.
- Get and list operations expose visible records only. Detail queries join the run's display name (its task title) and node-run history; list queries return lightweight summaries scoped to one project.
- Node-run history is read-only here — node-run creation, updates, and their state machine belong to the execution engine, which writes through the same repository.
- Soft deletion refuses active runs (a `Running` run, a non-terminal node run, or a `Running` session on the run's task) and then cascades a soft-delete across the run, its node runs, its task's sessions, worktrees, and task row in one transaction. The handler removes the physical worktree through the task worktree provisioner.
- Snapshot protection lives in the `workflow` module: a published snapshot referenced by a live run cannot be soft-deleted, and a workflow whose snapshots a live run freezes cannot be deleted either, so a run's frozen graph stays readable across its lifecycle.

`WorkflowRunRepository`, `WorkflowRunIdGenerator`, `TaskIdGenerator`, `WorktreeIdGenerator`, `TaskWorkflowProvisioner`, and `Clock` isolate storage, identity, provisioning, and time from the handlers. Creation parses the frozen graph to validate deployment prerequisites, capture the materialization receipt, and resolve the default kickoff input, but this module does not execute workflows or choose transport semantics — those belong to the [execution engine](engine/README.md) and transport adapters. `ora-backend` implements `NodeExecutor` as `WorkflowRunNodeExecutor` and wires it in `Backend::open`.

See the [ora-application overview](../../README.md) and [Workflow architecture doc](../../../../docs/workflow.md).
