# Project Application Module

This module owns the transport-independent project use cases and the shared `Clock` abstraction used across the application crate.

## Responsibilities and invariants

- `CreateProjectHandler` assigns a new `ProjectId`, timestamps the domain entity, and persists it through `ProjectRepository`.
- Get and list handlers return only repository-visible projects.
- `UpdateProjectHandler` changes the project name while preserving its id, root path, creation timestamp, and deletion state.
- Domain values are mapped to the shared project contract before leaving the application layer.
- Repository and not-found failures are normalized as `ApplicationError` and accompanied by structured operational events.

The module does not inspect the project root, create Git repositories, manage worktrees, or delete project aggregates. Backend composition and the database cascade path own those responsibilities.

`ProjectRepository`, `ProjectIdGenerator`, and `Clock` keep persistence, identity, and time injectable. Implementations must preserve the visible-record and soft-delete semantics expected by the handlers.

See the [ora-application overview](../../README.md) and [Application and Contracts Boundary](../../../../docs/application-contracts.md).
