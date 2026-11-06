# Application Boundary and Domain Models

English | [中文](application-boundary.zh.md)

Desktop's persisted operations are split across six owners so use-case orchestration, storage and
ACP streams stay independent of the UI shell. This document describes who owns what, the shape of
the public contract, the handler set and the invariants every caller can rely on.

## Ownership

| Owner           | Crate                    | Owns                                                                                                                                                                                                                             |
| --------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Domain          | `ora-domain`             | Schema-backed entities, identifier newtypes, categorical enums and the invariants that hold regardless of transport or storage                                                                                                   |
| Contracts       | `ora-contracts`          | Request, response, stream-event and public-error DTOs; TypeScript export into `@ora/contracts`                                                                                                                                   |
| Application     | `ora-application`        | Use-case handlers, `ApplicationError`, the repository/clock/identity/provisioning ports handlers depend on, domain-to-contract mapping                                                                                           |
| Persistence     | `ora-db`                 | SQLite implementations of those ports and schema reconciliation, see [Persistence](persistence.md)                                                                                                                               |
| Backend         | `ora-backend`            | Composition: SQLite bootstrap, the system clock, concrete repositories and handlers, the agent runtime host, plugin lifecycle, workflow execution, durable Git cleanup, public error projection and the shared request lifecycle |
| Desktop adapter | `apps/desktop/src-tauri` | Thin Tauri commands that accept a contract request, call a Backend domain handle or the Desktop filesystem service, and serialize the result, see [Desktop host](../desktop/desktop-host.md)                                     |

- `ora-contracts` keeps Rust field names idiomatic and serializes JSON in `camelCase`.
- ACP v1 wire types belong to the official `agent-client-protocol-schema` crate (Rust) and
  `@agentclientprotocol/sdk` (TypeScript). Contracts may embed them but never duplicate the ACP
  schema.
- The frontend endpoint catalog (operation names, client namespaces, DTO names, unary or stream
  mode) belongs to `xtask`; see [Frontend contracts](frontend-contracts.md).
- General filesystem browsing is deliberately outside `ora-backend`: Desktop composes the bounded
  workspace-file service in its command layer, see [Workspace files](../workspaces/workspace-files.md).

## Domain models

`ora-domain` has no dependency on SQLite, Tauri or `ora-contracts`. Depending on it is enough to
construct and inspect every model.

| Domain type                                                | Backing table                                                      | Notes                                                                                                                              |
| ---------------------------------------------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `Project`                                                  | `projects`                                                         | Name and repository facts; its checkout is the Main Workspace's location                                                           |
| `Workspace`, `WorkspaceProvisioning`                       | `workspaces`, `workspace_locations`, `workspace_provisioning`      | A stable execution environment shared by sessions and workflow runs; `Main` (the project checkout) or `Isolated` (a task worktree) |
| `Task`                                                     | `tasks`                                                            | The user-facing label of an isolated Workspace; it carries no status of its own                                                    |
| `Worktree`                                                 | `worktrees`                                                        | Optional one-to-one Git extension of a Workspace, keyed by the Workspace id                                                        |
| `Session`                                                  | `sessions`                                                         | One conversation in a Workspace and the provider session it currently runs on                                                      |
| `Skill`                                                    | `skills`                                                           | Namespaced Skill catalog entry                                                                                                     |
| `AgentDefinition`                                          | `agents`                                                           | A configurable agent: namespaced name, description and Markdown content                                                            |
| `Workflow`, `WorkflowSnapshot`                             | `workflows`, `workflow_snapshots`                                  | Editable draft plus immutable published snapshots, see [Workflows](../workflows/workflows.md)                                      |
| `WorkflowRun`, `WorkflowNodeRun`, `WorkflowExecutionScope` | `workflow_runs`, `workflow_node_runs`, `workflow_execution_scopes` | Executed runs, see [Workflow runs](../workflows/workflow-runs.md)                                                                  |
| `ScheduledTask`, `ScheduledRun`                            | `scheduled_tasks`, `scheduled_runs`, `scheduled_occurrences`       | See [Scheduled tasks](../scheduled-tasks/scheduled-tasks.md)                                                                       |
| `GitCleanupJob`                                            | `git_cleanup_jobs`                                                 | Durable physical Git cleanup, see [Tasks and worktrees](../workspaces/tasks-and-worktrees.md)                                      |
| `WorktreeProvisioningLease`                                | `worktree_provisioning_leases`                                     | Owner of Git resources while a task is being created                                                                               |

Workspace state is modelled with independent enums rather than optional fields: `WorkspaceKind`
(`Main`, `Isolated`), `WorkspaceLifecycle` (`Provisioning`, `Active`, `Unavailable`, `Retiring`,
`Deleted`), `WorkspaceLocation` (local filesystem, SSH or a remote target that carries only
references and opaque locators, never credentials) and `WorkspaceProvisioningState`.

### Identifiers

Identifiers are newtypes generated by one macro, so a `TaskId` cannot be passed where a `ProjectId`
is expected: `ProjectId`, `WorkspaceId`, `TaskId`, `ScheduledTaskId`, `ScheduledRunId`,
`GitCleanupJobId`, `WorktreeProvisioningLeaseId`, `SessionId`, `SkillId`, `AgentDefinitionId`,
`WorkflowId`, `WorkflowSnapshotId`, `WorkflowRunId`, `WorkflowScopeId` and `WorkflowNodeRunId`.
Each wraps a `String` and serializes as that bare string. A `Worktree` uses its owning `WorkspaceId`
as its identity.

### Categorical fields

Closed categories are enums, never raw codes: for example `SessionStatus` (`Running`, `Stopped`)
and `WorktreeActivity` (`Inactive`, `Active`). Each exposes `database_value()` and a
`from_database_value()` that rejects unknown persisted values with `DomainModelError`; there is no
implicit conversion from integers.

An agent identity is deliberately not a closed enum. `AgentRef` is a validated newtype over the
canonical plugin id of the package that supplies the agent (for example
`official/ora-space.claude`), because every agent comes from an installed plugin and which ones
exist is not knowable at build time. `AgentRef::parse` rejects only a blank value; an unrecognized
identity means "not installed right now", not corrupt data.

### Validation in the domain

Most models are plain snapshots whose constructors cannot fail. The exceptions:

- `Skill::new` trims the name and rejects a blank one. The name must be a single filesystem-safe
  segment (ASCII alphanumerics plus `.`, `_`, `-`, at most 255 bytes, not starting with `.`), so a
  committed Skill can never be promoted onto a reserved or hidden directory under the Skills root.
- `AgentDefinition::new` trims the name and rejects a blank one.
- `SessionTitle::parse` trims agent-provided text, rejects blank values and more than 255 Unicode
  scalar values without truncating. `Session.title` is `Option<SessionTitle>`.

`Session.workspace_id` is fixed at construction because it decides the working directory. Its
`agent_ref` and `agent_session_id` are the current binding rather than its identity:
`with_binding` replaces both when a conversation moves to another agent, while the id and recorded
history continue. A `Session` cannot exist before the provider returns its session id.
`HistoryState` is an independent axis persisted as one nullable reason column: absent means
`Writable`, present means `Degraded` with the reason the user has to act on.
`SessionMcpSelection` is either `Automatic` or an explicit set of plugin ids, see
[Session MCP](../agents/session-mcp.md).

`AuditFields` carries `created_at`, `updated_at` (Unix milliseconds) and `is_deleted` for every
soft-deletable entity. Optional fields exist only where the column is nullable.

## Contract shapes

Contracts are the app-facing protocol, not a projection of the domain. Each entity has one public
view model reused across create, get, list and update responses, for example:

- `Project`: `id`, `name`; `CreateProjectRequest` takes `name` and `mainWorkspacePath`
- `Task`: `id`, `projectId`, `workspaceId`, `title`
- `Session`: `id`, `workspaceId`, `title`, `agentRef`, `status`, `historyState`, `mcpSelection`
- `Skill`: `id`, `namespace`, `name`, `description`, `source` (local or plugin), `availability`
- `Agent`: `id`, `namespace`, `name`, `description`
- `ProjectBranch`: `name`, `refName`, `displayName`

Public payloads expose documented business fields only; `isDeleted` and other audit fields never
appear. Workflow summary and snapshot timestamps are deliberate exceptions because version history
and editor freshness are user-visible.

- Task worktrees are backend-owned: no contract carries a worktree id and there are no worktree
  operations. `CreateTaskRequest` takes `projectId`, `title` and `baseBranch`.
- A session's private provider session id is persisted and used for `session/load` but never
  exposed.
- Workspace file contracts carry the task or project identity and normalized relative paths; the
  server resolves the root and callers never supply one.
- Workspace diff contracts expose a patch snapshot (`baseCommitId`, `headCommitId`, `patch`) and
  carry neither filesystem paths nor Git diagnostics.

## Public errors

A public failure serializes directly as `{ code, params, requestId }`. `RequestId` is a UUID and
`PublicError` is a discriminated union with one parameter shape per code (including explicitly
empty ones). There is no internal message and no outer envelope.

The backend maps the highest semantic `ApplicationError` exhaustively to a public error and to one
transport-neutral classification: `InvalidRequest`, `PayloadTooLarge`, `NotFound`, `Conflict`,
`Unprocessable`, `Forbidden`, `HostUnavailable` or `Internal`. Infrastructure failures become
`internal_error` while their `Error::source()` chain stays available for logs. Adapters never infer
public codes from source chains or error strings. How each classification is logged is described in
[Logging](logging.md).

## Handlers

Every `ora-application` handler accepts exactly one request contract and returns exactly one
response contract or an `ApplicationError`; none references a UI, Tauri or database type.
Dependencies are statically dispatched through generics, so a unit test can build any handler with
in-memory fakes.

| Module             | Use cases                                                                                                            |
| ------------------ | -------------------------------------------------------------------------------------------------------------------- |
| `project`          | create, get, list, list branches, update                                                                             |
| `task`             | create, get, list, update                                                                                            |
| `session`          | get, list, delete                                                                                                    |
| `skill`            | create, get, list, update, delete, startup reconciliation                                                            |
| `skill_import`     | prepare, get, commit, cancel                                                                                         |
| `agent_definition` | create, get, list, update, delete; Markdown import (prepare, commit)                                                 |
| `effect`           | Desired State and Target status, see [Effects](../agents/effects.md)                                                 |
| `workspace_diff`   | read diff and status, stage, unstage, commit, push                                                                   |
| `workflow`         | create, get, list, update, delete, analyze, draft get/update, publish, rollback, activate, versions, delete snapshot |
| `workflow_run`     | CRUD plus the run engine (start, cancel, restart, resume, node completion)                                           |
| `scheduled_task`   | create, get, list, list history, update, delete, revoke approval                                                     |
| `worktree`         | none, ports only                                                                                                     |

Consequences of that set:

- Project and Task have no delete handler. Aggregate deletion is a transactional cascade owned by
  `ora-backend` and `ora-db`, because it must reject running descendants and update several tables
  atomically.
- Session creation, prompting, permissions, cancellation, agent switching and history recording
  belong to the agent runtime, not to `ora-application`; the session module supplies reads and soft
  deletion only. See [Agent runtime](../agents/agent-runtime.md).
- `workspace_diff` owns review use cases for any checkout, but not workspace selection: backend
  composition resolves the live cwd and, when a `Worktree` row exists, its fixed baseline. See
  [Workspace review](../workspaces/workspace-review.md).
- `project_id`, `task_id` and `workspace_id` are pass-through identifiers: create and update handlers
  do not run extra existence checks before delegating to their repositories.
- Deletion is a normal delete use case at the boundary even though the repository soft-deletes.

### Agent import

`prepareAgentImport` parses a Markdown agent file (at most 1 MiB, YAML front matter with `name` and
`description`) and reports whether a `local` agent with the same name exists. `commitAgentImport`
re-parses the content and imports it, or, on a name conflict, applies the caller's explicit
`skip` or `overwrite` decision. The caller passes the conflicting agent's id and `updatedAt` from
preparation; if the stored row changed in between, the result is `stale_conflict` and nothing is
written.

## Invariants

- A project's Main Workspace location is fixed at creation; `UpdateProjectHandler` renames only.
- Project creation rejects a local Main Workspace path that is blank or not an existing directory
  with `project_workspace_not_directory`; a relative path is resolved against the same base that
  later cwd resolution uses. A local Main Workspace is `active` from creation: Ora never provisions
  a local checkout, so whether its directory still exists is re-checked on every use and reported
  as `workspace_unavailable` instead of being frozen into the lifecycle.
- A session's Workspace is immutable. Lifecycle operations change only `status` and `updated_at`.
- `UpdateTaskRequest` cannot change project or Workspace ownership.
- Project and Task deletion soft-delete the whole Ora-owned aggregate in one SQLite transaction. A
  running Session rejects the operation with `resource_in_use`. These paths never call Git and never
  delete provider-owned ACP history, but they do remove the history Ora recorded.
- Physical Git cleanup after deletion or a failed creation is durable and asynchronous, see
  [Tasks and worktrees](../workspaces/tasks-and-worktrees.md).

## Error propagation and completion logging

Handlers and repositories preserve semantic errors and source chains without emitting generic
success or propagation-only failure events. The Tauri command and channel entry points own the
single correlated request-completion event and derive its level from the public classification.
Bootstrap, migration, state-transition and cleanup events remain independent lifecycle facts. See
[Logging](logging.md).
