# Application and Contracts Boundary

The first `project` vertical slice is split across `ora-application`, `ora-contracts`, and transport adapters so the repository can prove an end-to-end flow without coupling use-case orchestration to HTTP or Tauri.

## Ownership

- `ora-contracts` owns serialization-friendly request and response DTOs for `CreateProject`, `GetProject`, `ListProjects`, `UpdateProject`, and `DeleteProject`.
- `ora-contracts::Project` is the single shared app-facing project payload for the first slice. It exposes `id`, `name`, and `root_path` only.
- `ora-contracts` keeps Rust field names idiomatic while serializing JSON payloads in `camelCase` for adapter and frontend consumption.
- `ora-application` owns project CRUD handlers, application errors, repository ports, and the mapping from `ora-domain::Project` into `ora-contracts::Project`.
- Transport adapters such as `apps/web/server` stay thin: they accept contract requests, delegate to `ora-application`, and return contract responses or application errors.

## Project Slice Notes

- The current implementation keeps delete externally CRUD-shaped through `DeleteProjectHandler`.
- Repository implementations can still soft-delete internally by updating `is_deleted` and `updated_at`.
- `ora-db` now provides SQLite-backed implementations of the `ora-application` repository ports for `project`, `task`, `session`, and `worktree`.
- `ora-application` emits structured operational `tracing` events for project CRUD handlers with an `operation` field and, when available, a `project_id`. Success events log at `INFO`, and not-found or repository failures log at `ERROR` with failure details under `error`.
- The application layer emits events only; logging initialization, sink selection, and writer lifetimes stay owned by runtime composition roots such as `apps/web/server`.
