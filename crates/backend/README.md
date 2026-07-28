# ora-backend

`ora-backend` is the transport-neutral composition root shared by Web and Tauri adapters. It opens persistent state, wires concrete application repositories and handlers, supervises agent providers, and exposes one stable `Backend` API over contract DTOs.

## Responsibilities

- `Backend::open` creates required directories, bootstraps and migrates SQLite, constructs CRUD APIs, and starts the [agent runtime](src/agent_runtime/README.md).
- Project, task, skill, and agent operations delegate to `ora-application`; aggregate deletion uses transactional database cascades followed by best-effort cleanup of Ora-owned Git state.
- Session creation, loading, prompting, permissions, stopping, deletion, and model discovery delegate to the agent runtime.
- `BackendError` converts internal failures into stable codes and transport-neutral categories. HTTP and Tauri adapters map those categories into their native error semantics.
- The configured worktree root affects task creations that begin after an update and supplies the deterministic-path fallback used by aggregate Git cleanup. Runtime Session paths continue to resolve from persisted branch identity and Git metadata.

## Ownership boundaries

Project and task deletion soft-delete Ora-owned database records in one transaction and reject aggregates with running sessions. A successful transaction returns the exact task, repository, and branch identities it deleted; the backend then delegates each validated target to `TaskGitResourceCleaner`. The cleaner resolves a linked worktree by branch first and by exact `worktree_root/task_id` only as a fallback, explicitly refusing the main checkout, before independently deleting the validated local branch. Missing Git resources count as already cleaned, while other Git failures are logged independently and never change the successful database response. Cleanup has no durable retry or completion mechanism.

Task creation and aggregate deletion share a short synchronous lifecycle lock so a cascade cannot miss Git state being provisioned concurrently. Session creation, load admission, and aggregate deletion also share an asynchronous lifecycle lock. Create holds it through Running persistence, and load acknowledges admission only after the same durable transition, so a cascade cannot delete the owning checkout during provider setup. Session deletion remains owned by the agent runtime, and aggregate deletion does not delete provider-owned ACP history.

`ProjectWorkContext` and filesystem browsing remain outside this crate. Logging initialization and environment parsing belong to runtime composition roots, while this crate only emits events through shared logging APIs.

Dropping the last backend owner shuts down provider supervisors and initiates bounded process-tree cleanup.

See [Application and Contracts Boundary](../../docs/application-contracts.md) and [ACP Agent Runtime](../../docs/agent-runtime.md).
