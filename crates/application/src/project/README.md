# Project Application Module

This module owns the transport-independent project use cases and the shared `Clock` abstraction used across the application crate.

## Responsibilities and invariants

- `CreateProjectHandler` assigns a new `ProjectId`, timestamps the domain entity, and persists it through `ProjectRepository`.
- Get and list handlers return only repository-visible projects.
- `ListProjectBranchesHandler` joins local branch refs with project-owned task and worktree records so Ora-managed branches use task titles.
- `UpdateProjectHandler` changes the project name while preserving its id, root path, creation timestamp, and deletion state.
- Domain values are mapped to the shared project contract before leaving the application layer.
- Repository and not-found failures are normalized as `ApplicationError`; request lifecycle adapters own correlated completion events.

The module does not execute Git commands, create Git repositories, manage worktrees, or delete project aggregates. Backend composition implements `BranchLister`, while the database cascade path owns project deletion.

`ProjectRepository`, `BranchLister`, `ProjectIdGenerator`, and `Clock` keep persistence, Git inspection, identity, and time injectable. Implementations must preserve the visible-record, local-ref, and soft-delete semantics expected by the handlers.

See the [ora-application overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts-boundary.md).
