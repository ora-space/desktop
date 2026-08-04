# workflow-run/runtime

Ports and in-memory mock for project workflow mounts and graph runs.

## Boundaries vs other stacks

- **Not** the settings React Flow definition editor (deploy calls into Host/
  Run from settings chrome; graph editing stays there).
- **Not** OpenSpec `workflow-store` / Spec stepper — types are named
  `GraphWorkflowRun*` to avoid collisions; do not mix Spec-mode writes here.
- **This package slice** owns mock Host/Run repositories and the event stream
  consumed by Theater / Overview in the parent feature.

## Responsibilities

- Define `WorkflowHostRepository` / `WorkflowRunRepository` and shared run types.
- Provide a memory implementation that registers `DemoWorkflow` snapshots,
  mounts them to projects, and creates runs with frozen definition snapshots.
- Drive runs with `mock-run-engine` + `mock-execution-plan` (timed progression and
  `WorkflowRunEvent` stream). Cancel / delete clear timers so mid-run stop is reliable.
- Expose the active runtime through React context for hooks and UI.
  The default memory instance is process-lifetime for the provider mount —
  language switches must not recreate it (that would wipe mounts/runs).
- Notify `runs.watch` listeners on mutations so react-query can refresh sidebar
  status without Theater UI.

## Non-responsibilities

- No HTTP/NDJSON transport (Follow-up F2).
- No Theater / overview UI (parent `workflow-run` feature owns forms/toasts /
  result act).
- Not the settings session graph editor.
- Settings **Test run** still uses `@ora/workflow-mock` `runDemoWorkflow` — a
  separate demo path until an optional later convergence.
- HITL `fail` / `skip` timeout policies are reserved on the type; MVP uses
  `wait` and never auto-times out.
- No `partial_failed` aggregation — status exists on the type for UI
  placeholders; the mock finish path does not synthesize it.

## Invariants

- Creating a run freezes `definitionSnapshot` so later library edits cannot
  mutate an in-flight or historical run.
- While `pending`, `updateSnapshotNode` may patch `description` /
  `instruction` on that run's snapshot node copy only. Never writes back to
  the mounted library definition; rejects once the run has started.
- Mount is unique per `(projectId, definitionId)`; remount upserts. Multiple
  executions are separate `GraphWorkflowRun` rows.
- Concurrent runs are independent; cancelling one does not stop siblings.
- Event union shapes must stay mappable to future NDJSON frames.
- Types use `GraphWorkflowRun` naming to avoid colliding with OpenSpec
  `WorkflowRun` in `workflow-store`.

## Mock engine semantics (extensible)

- **Path plan**: `planMockExecution` walks from `start` seeds. At `condition`
  nodes it picks **one** outgoing edge via `MockPathPolicy` (default is
  kickoff-aware label heuristics; otherwise first edge). Unreachable nodes are
  marked `skipped` and emit `node_finished` with that status.
- **Scheduling**: ready-set waves — every reachable node whose predecessors
  have succeeded starts together (true fan-out parallelism). Deploy library
  workflow **错开并行演示 / Staggered parallel demo** for unequal start/end
  times (`data.mockStepMs`), or **并行审查演示 / Parallel review demo** for a
  synchronized fan-out (Theater shows one card at a time with a parallel switcher).
- **Start**: only from `pending`. Re-entrant `start` is a no-op; HITL resume
  uses `submitHitl`.
- **HITL**: `prompt` kind nodes append an open request to `openHitls` and set
  the node to `awaiting_input`, emitting `hitl_required`. Each request carries
  `schema.kind` (`approval` | `feedback` | `clarify`), optional `schema.prompt`
  (model/engine question body), `blocking`, `createdAt`, and `fields` (submit
  payload keys = `field.name`). Schema copy follows engine `locale`. Multiple
  prompts may wait concurrently; the user can resolve any gate by `requestId`
  and may browse other acts while collapsed. Submit validates required fields
  and select membership, emits `hitl_resolved` with `payload` + `nodeId`,
  succeeds that node (with I/O summaries), and pumps. Cancel clears `openHitls`.
  Policy is `wait` (no auto-timeout); `timeoutAt` is reserved for UI when
  policy is not `wait`.
- **Node I/O**: each node state may carry glanceable `input` / `output`
  (`summary` + optional `detail`) for the act inspector.
- **Tokens**: stubbed for `prompt` (on HITL submit) / `agent` / `tool` kinds.
- **Artifacts**: markdown stubs on `agent` / `output`.
- **Options**: `nodeStepMs` (default 5000), `autoStart` (default false;
  workspace Start calls `runs.start`), injectable `pathPolicy`.
- Kickoff text is stored on the run and fed into path planning when provided.
  Deploy creates a pending run only; the workspace Start control begins execution.
