# Workflow Application Module

This module implements transport-independent CRUD and lifecycle use cases for workflow definitions and their versioned snapshots.

## Responsibilities and boundaries

- Creation assigns `WorkflowId` and `WorkflowSnapshotId`, applies backend timestamps, validates the domain entity, and atomically persists the workflow together with its initial draft.
- Get and list operations expose visible records only. Detail queries join the draft and published snapshot; list queries return lightweight summaries without graph data, newest created first.
- Draft updates mutate the draft's graph in-place without creating a new snapshot row.
- Publish validates stable IDs and graph structure, then copies the draft into an immutable
  snapshot and activates it in a single repository-managed transaction. The SQLite adapter repeats
  validation inside that transaction so a concurrent draft write cannot publish invalid data.
- Rollback copies a historical snapshot's graph into the draft without changing the published pointer.
- Activate switches the published pointer to a different snapshot and syncs its graph into the draft.
- Version listing returns published snapshot metadata only (no graph content). Individual version lookups return the full snapshot.
- Workflow deletion cascades a soft-delete to all associated snapshots. Snapshot deletion refuses to delete the draft or the currently active version.
- Domain validation and repository errors are translated into stable `ApplicationError` variants, which the backend projects into the shared `PublicError` contract.

`WorkflowRepository`, `WorkflowIdGenerator`, and `Clock` isolate storage, identity, and time from the handlers. The module maps domain entities to contract DTOs and validates publishable graph structure, but does not execute workflows or choose transport semantics.

See the [ora-application overview](../../README.md) and [Workflow architecture doc](../../../../docs/workflow.md).
