# @ora/workflow-runtime

Transport-neutral Host/Run ports and an in-memory adapter for **graph workflow runs**.

## Responsibilities

- Define `WorkflowHostRepository` / `WorkflowRunRepository` and shared run types
  (`GraphWorkflowRun*`, HITL, artifacts, `WorkflowRunEvent`).
- Normalize editor graphs into serializable `WorkflowDefinition` snapshots so
  execution contracts never depend on React Flow runtime objects.
- Derive a stable display path order (`workflowPathOrder`) for consumers that
  need a linear node list: topological constraints first, canvas position as
  the ready-set tie-break — without rewriting the frozen snapshot array.
- Validate deployable definitions as non-empty DAGs with unique identifiers and
  resolvable edges, preventing invalid graphs from entering a stuck run state.
- Provide `createMemoryWorkflowRuntime` that mounts `DemoWorkflow` snapshots,
  creates frozen-definition runs, and drives timed mock progression.
- Attach sequence/cursor/timestamp metadata to events and pair initial state
  with a live cursor so the Desktop channel adapter can reconnect without gaps.

## Non-responsibilities

- No React / Theater / Overview UI (owned by `app-shell` `workflow-run`).
- No generated-contract adapter yet (Follow-up F2).
- Not the settings React Flow definition editor.
- No HITL `fail` / `skip` auto-timeout execution; MVP policy is `wait`.
- No `partial_failed` aggregation — status exists for UI placeholders.

## Boundaries

| Package                  | Owns                                                 |
| ------------------------ | ---------------------------------------------------- |
| `@ora/workflow-mock`     | React Flow fixtures, editor validation, session demo |
| `@ora/workflow-runtime`  | Execution DTOs, mounts, runs, events, memory adapter |
| `app-shell` workflow-run | Theater / Overview / hooks / React context           |

## Public API

Import stable DTOs, ports, normalization, and HITL helpers from
`@ora/workflow-runtime`.

Import `createMemoryWorkflowRuntime` and mock engine/testing helpers from the
explicit `@ora/workflow-runtime/memory` adapter entry. Production code outside
the composition root must not depend on that subpath.

Inject the runtime into the shell via `WorkflowRuntimeProvider` (React context
stays in app-shell so this package remains UI-free).

## Planned backend adapter

Workflow will follow the same stack as project/task/session: Rust domain and
application handlers persisted in SQLite, DTOs generated from `crates/contracts`,
and operations delivered through the Desktop contracts client and Tauri
commands/channels. The adapter will preserve typed errors, `AbortSignal`, bounded
stream queues, and request correlation.

The future adapter will implement these ports around `ContractsClient.workflow`.
The host passes it through `AppShell.workflowRuntime`; the current memory adapter
remains the explicit prototype default until those contracts exist.

`getLiveSnapshot` returns an atomic run/artifact snapshot plus a cursor.
`subscribe(..., { afterCursor })` replays later envelopes before live delivery,
which closes the initial-query/subscription race and provides the same semantics
the channel adapter must preserve. The handoff registers live delivery before
replay and queues interleaving events, so observer-triggered synchronous writes
cannot create a gap or reorder the stream. UI caches advance the cursor for
every envelope and therefore do not repeat historical side effects on remount.

State-changing commands return the latest `GraphWorkflowRun`. Real adapters
should return that snapshot in the command response rather than forcing a
follow-up GET; stream events remain the source for subsequent progress.
