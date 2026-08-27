# Application and Contracts Boundary

The public application surface is split across `ora-domain`, `ora-contracts`, `ora-application`, `ora-db`, `ora-backend`, and the Desktop Tauri adapter so use-case orchestration and ACP streams remain independent of the UI shell.

## Ownership

- `ora-domain` owns schema-backed entities, identifier newtypes, and categorical enums. See [Domain Models](domain-models.md).
- `ora-contracts` owns serialization-friendly request, response, stream-event, and public-error
  DTOs for Project, Task, Task Diff review, Spec management, Session, Skill, Skill Import, Effect,
  Agent, Workflow, Git identity, and workspace-file operations.
- `ora-contracts` keeps Rust field names idiomatic while serializing JSON payloads in `camelCase` for adapter and frontend consumption.
- ACP v1 wire types are owned by the official `agent-client-protocol-schema` crate in Rust and `@agentclientprotocol/sdk` package in TypeScript. `ora-contracts` may embed those types in Ora application DTOs, but does not duplicate the ACP schema.
- The `xtask` exporter owns the generation-only frontend endpoint catalog: operation names, client namespaces, request and response types, and unary-versus-stream response mode.
- `ora-contracts` exports TypeScript DTOs into `packages/contracts/src` so frontend packages consume the contract surface from `@ora/contracts`. See [Frontend Contract SDK](frontend-contract-sdk.md).
- `ora-application` owns use-case handlers, `ApplicationError`, the repository/clock/identity/provisioning ports those handlers depend on, and domain-to-contract mapping.
- `ora-db` implements those ports on SQLite and owns schema reconciliation. See [Database Repositories](database-repositories.md).
- `ora-backend` owns SQLite bootstrap, the system clock, concrete repository and handler composition, task-diff workspace/baseline resolution, specification target/configuration/filesystem composition, transactional aggregate deletion, dynamic project selection for task Git operations, one application-scoped supervisor per supported agent CLI, grouped model discovery, per-session ACP routing, transport-neutral public error projection, and the shared request lifecycle used by runtime adapters.
- The Tauri adapter stays thin: commands accept contract requests, delegate to the same `Backend` or Desktop filesystem service, then serialize stable public errors over IPC.
- General filesystem browsing is deliberately outside `ora-backend`; Desktop composes the bounded workspace-file service in its command layer, while specification access remains part of backend composition because it combines persisted targets with discovery.

## Contract shapes

Contracts are the app-facing protocol, not a projection of the domain. Each entity has one shared public view model reused across create, get, list, and update responses instead of separate summary and detail variants:

- `Project`: `id`, `name`, `rootPath`
- `Task`: `id`, `projectId`, `workspaceId`, `title`
- `Session`: `id`, `taskId`, `agentCli`, `status`, `historyState`
- `Skill`: `id`, `namespace`, `name`, `description`, `availability`
- `WorkspaceEffect`: `workspaceId`, `generation`, and the complete normalized desired Skill set;
  surface status exposes desired/observed/applied generations plus structured current conditions.
- `Agent`: `id`, `namespace`, `name`, `description`
- `ProjectBranch`: `name`, `refName`, `displayName`
- Workspace file contracts keep task identity in the request and expose only normalized relative paths: `WorkspaceEntry`, `ReadWorkspaceFileResponse`, `SearchWorkspaceResponse`, and `WorkspaceFileEventBatch`. The server resolves the task's managed workspace; callers never provide a filesystem root.
- `Workflow`: `id`, `name`, `publishedSnapshotId`
- `WorkflowSnapshot` (public): `id`, `workflowId`, `version`, `graph`, `createdAt`, `updatedAt`
- `WorkflowSummary`: `id`, `name`, `publishedVersion`, `createdAt`, `updatedAt`
- `WorkflowVersion`: `id`, `version`, `createdAt`
- `WorkflowRun`: `id`, `workflowId`, `snapshotId`, `status`, `state`, `input`, `output`, `error`, `startedAt`, `finishedAt`, `createdAt`, `updatedAt`
- `WorkflowNodeRun`: `id`, `runId`, `nodeId`, `nodeType`, `sessionId`, `status`, `input`, `output`, `error`, `startedAt`, `finishedAt`, `createdAt`, `updatedAt`
- `WorkflowRunSummary`: `id`, `name`, `projectId`, `workflowId`, `status`, `startedAt`, `finishedAt`, `createdAt`

Public payloads expose documented business fields only. `isDeleted` and other internal audit fields never appear. Workflow summary timestamps and WorkflowSnapshot `createdAt`/`updatedAt` are explicit exceptions because version history and editor freshness are user-visible lifecycle facts. `createdAt` records when a snapshot was created; `updatedAt` records draft edits and remains `null` for published snapshots. Two exclusions are deliberate:

- Task worktrees are backend-owned, so no contract carries a separate `worktreeId`, and there are no standalone worktree DTOs or SDK operations. `CreateTaskRequest` takes `projectId`, `title`, and `baseBranch`; `UpdateTaskRequest` takes `taskId` and `title`. See [Task Worktrees](task-worktrees.md).
- A session's private provider session id is never exposed. It is persisted and used internally for `session/load`, but the public `Session` payload omits it.

Task Diff contracts expose a patch snapshot (`baseCommitId`, `headCommitId`, `patch`). The contract intentionally carries neither filesystem paths nor Git command diagnostics.

Because adapters and generated frontend types bind to `ora-contracts` rather than to domain models, the domain layer can add internal fields or invariants without those details leaking into the protocol.

## Public errors

Public failures serialize directly as `{ code, params, requestId }`. `RequestId` is UUID-backed, and `PublicError` is a discriminated union with one named parameter shape per code, including explicit empty parameters. The contract exposes neither an internal message nor an outer error envelope.

The backend maps the highest semantic application error exhaustively to both a public error and one transport-neutral classification: `InvalidRequest`, `PayloadTooLarge`, `NotFound`, `Conflict`, `Unprocessable`, or `Internal`. Workspace Diff exposes `workspace_diff_commit_message_blank` for blank commit messages from application handlers, plus `workspace_diff_baseline_unavailable` and `workspace_diff_too_large` when `WorkspaceDiffApi` cannot resolve a baseline (requested by the `Branch`/`Committed` scopes only — a workspace with no recorded `Worktree` row, such as a project's main checkout, simply has none) or the bounded patch exceeds the response limit. Infrastructure failures become `internal_error` while their Rust `Error::source()` chains remain available for diagnostics outside the serialized contract. Adapters never infer public codes by inspecting source chains or matching error strings. Skill upload limits and folder conflicts expose only bounded safe parameters such as `maxBytes`, `maxFiles`, and a validated destination name.

## Handlers

`ora-application` handlers are adapter-agnostic entry points. Each accepts exactly one request contract type and returns exactly one response contract type or an `ApplicationError`, referencing no UI, Tauri, or database type. A Tauri command can therefore deserialize transport input into one contract value, call the handler, and serialize the result with no extra use-case orchestration in the adapter.

Handlers own their ports, so a unit test can construct any handler with in-memory fakes and execute the full use case with no database, Git process, or Tauri runtime. Dependencies are statically dispatched through generics rather than trait objects.

The handler set is intentionally narrower than full CRUD per entity, because some lifecycles are owned elsewhere:

| Module             | Handlers                                                                                                                        |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `project`          | create, get, list, list branches, update                                                                                        |
| `task`             | create, get, list, update                                                                                                       |
| `session`          | get, list, delete                                                                                                               |
| `skill`            | create, get, list, update, delete, startup reconciliation                                                                       |
| `skill_import`     | prepare, get, commit, cancel batch sessions                                                                                     |
| `effect`           | get desired, replace desired with generation CAS, get surface status, retry surface                                             |
| `agent_definition` | create, get, list, update, delete                                                                                               |
| `workspace_diff`   | read diff, commit, push                                                                                                         |
| `workflow`         | create, get, list, update, delete, getDraft, updateDraft, publish, rollback, activate, listVersions, getVersion, deleteSnapshot |
| `workflow_run`     | create, get, list, listNodeRuns, delete                                                                                         |
| `worktree`         | none — ports only                                                                                                               |

Notable consequences:

- There is no delete handler for `project` or `task`. Aggregate deletion is a transactional cascade owned by `ora-backend` and `ora-db`, because it has to reject running descendants and update several tables atomically.
- `ListProjectBranchesHandler` joins local Git refs with project-owned task and worktree records so an Ora-managed branch keeps its resolvable ref while displaying the owning task title.
- Session creation, load, prompt, permission response, cancellation, stop, agent switching, and history recording belong to the backend agent runtime, not to `ora-application`. The session module supplies only the persistence-facing reads and soft deletion.
- `worktree` has no handlers or transport contracts at all. Worktree records are internal metadata coordinated by the task module.
- `workspace_diff` owns review use cases for any workspace checkout — an isolated task worktree or a project's main checkout — but not workspace selection. Backend composition resolves the workspace's live cwd and, when a `Worktree` row is recorded for it, supplies its fixed baseline; a workspace with no such row (a project's main checkout) has no baseline, so only the `Unstaged`/`Staged` diff scopes apply and writes go through unverified.
- `workflow` offers a complete CRUD surface including deletion, unlike project and task. Workflow deletion follows the standard handler pattern because it has no running-session constraint; cascade soft-deletion of snapshots is managed entirely within the repository.
- `workflow_run` deletion carries an active-run guard — a `Running` run, a non-terminal node run, or a `Running` session bound to one of the run's node runs refuses deletion — and cascades a soft-delete across the run, its node runs, and those node-owned sessions. A not-started `Pending` run is not active. The shared Workspace is never deleted with one run.

`project_id`, `task_id`, and `workspace_id` are treated as pass-through business identifiers. Create and update handlers do not perform extra cross-entity existence checks before delegating to their repositories.

Deletion stays a normal delete use case at the boundary even though the repository implements it as a soft delete. Callers interact with delete-oriented request and response contracts and never see soft-delete or archive semantics.

## Error propagation and completion logging

Application handlers preserve semantic errors and infrastructure source chains without emitting generic success or propagation-only failure events. Repository adapters likewise do not add another event merely to report the same failure. Tauri command and channel entry seams own the single correlated request-completion event and derive its level from the public classification. Workspace watcher failures are converted to the same `{ code, params, requestId }` contract frame and complete the request lifecycle exactly once.

Bootstrap, migration, state-transition, and secondary-cleanup events remain independent lifecycle facts. Logging initialization, sink selection, and writer lifetimes stay with runtime composition roots. See [Runtime Logging](runtime-logging.md).

## Slice invariants

- Project roots are immutable after creation; `UpdateProjectHandler` renames only.
- A session's task is immutable, because it decides the working directory the conversation lives in. Its provider CLI and provider session id are the current binding rather than its identity: `switchSessionAgent` replaces both while the identifier and the recorded history continue. Ordinary lifecycle operations still change only `status` and `updated_at`.
- `UpdateTaskRequest` cannot change project or Workspace ownership.
- Project and Task deletion soft-delete the complete Ora-owned aggregate in one SQLite transaction. A running Session rejects the operation with `resource_in_use`; stopped children are cascaded. These paths never call Git and never delete provider-owned ACP history, but they do remove the session history Ora itself recorded — see [ACP Agent Runtime](agent-runtime.md).
- Task creation resolves the requested project's Git root at creation time and commits its rows through a project-visibility-validating unit of work guarded by a provisioning lease. Deletion soft-deletes Ora database records and, in the same transaction, registers durable Git cleanup jobs; the backend worker then removes the linked worktree and its `ora/*` branch asynchronously with at-least-once, idempotent execution.
- Worktree task creation resolves the selected local base ref to an immutable commit. Branch listing and creation do not fetch or merge remote-tracking refs.
- Worktree paths are composed only when creating a new worktree. Existing paths are resolved from the persisted branch name and Git's authoritative metadata, never reconstructed from the configured creation root.
- Task diffs compare against the isolated worktree's recorded creation commit.
- Workspace file paths are validated as relative paths, canonicalized before containment checks, and returned with slash separators. The filesystem crate is read-only, bounds file reads and search output, and reports native watcher changes as cache-invalidating batches. See [Task Workspace Files](task-workspace-files.md).
