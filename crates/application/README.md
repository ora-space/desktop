# ora-application

`ora-application` is Ora's transport- and storage-independent use-case layer. It coordinates domain models, contract DTOs, repositories, clocks, identifier generators, and Git worktree provisioning without depending on SQLite, HTTP, or Tauri.

## Module map

- [agent_definition](src/agent_definition/README.md) manages configurable agent definitions.
- [project](src/project/README.md) manages project creation, reads, and rename operations.
- [project_work_context](src/project_work_context/README.md) owns window-to-project leases and occupancy rules.
- [session](src/session/README.md) provides persisted session reads and soft deletion.
- [skill](src/skill/README.md) manages reusable skill records.
- [task](src/task/README.md) coordinates task persistence and optional Git worktree creation.
- [worktree](src/worktree/README.md) defines persistence and identity ports for task-owned worktrees.

## Boundaries

Handlers accept `ora-contracts` requests, operate on `ora-domain` models, and map results back to contract responses. Infrastructure is injected through statically dispatched repository, clock, identifier, and worktree traits so use-case rules can be tested without a database or Git process.

`ApplicationError` is the stable application-facing failure vocabulary. Handlers translate domain validation and infrastructure failures into this vocabulary and emit structured operational events, but they do not choose logging sinks or transport status codes.

Aggregate deletion, SQLite composition, ACP process supervision, and transport-neutral public error normalization belong to `ora-backend` and `ora-db`. Contract serialization and endpoint metadata belong to `ora-contracts`.

See [Application and Contracts Boundary](../../docs/application-contracts.md) for the cross-crate ownership model.
