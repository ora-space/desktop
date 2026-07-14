# Application and Contracts Boundary

The first `project` vertical slice is split across `ora-application`, `ora-contracts`, and transport adapters so the repository can prove an end-to-end flow without coupling use-case orchestration to HTTP or Tauri.

## Ownership

- `ora-contracts` owns serialization-friendly request and response DTOs for `CreateProject`, `GetProject`, `ListProjects`, `UpdateProject`, `DeleteProject`, `OpenProjectWorkContext`, and `RenewProjectWorkContext`.
- `ora-contracts::Project` is the single shared app-facing project payload for the first slice. It exposes `id`, `name`, and `root_path` only.
- `ora-contracts` keeps Rust field names idiomatic while serializing JSON payloads in `camelCase` for adapter and frontend consumption.
- `ora-contracts` also owns the frontend endpoint manifest for the exported HTTP CRUD surface, including operation names, HTTP methods, path templates, path parameters, request types, response types, and JSON body behavior.
- `ora-contracts` exports TypeScript DTOs plus the generated frontend SDK into `packages/contracts/src` so frontend packages can consume the generated contract surface from `@ora/contracts` and the browser transport from `@ora/contracts/fetch`.
- Backend-owned task worktrees stay internal; `ora-contracts` does not export standalone public worktree CRUD DTOs, SDK operations, or task payload linkage fields for them.
- `ora-application` owns project CRUD handlers, application errors, repository ports, and the mapping from `ora-domain::Project` into `ora-contracts::Project`.
- `ora-application` also owns the project work context handlers, lease timing rules, occupancy conflicts, and the mapping from `ora-domain::ProjectWorkContext` into the shared contract payload.
- Transport adapters such as `apps/web/server` stay thin: they accept contract requests, delegate to `ora-application`, and return contract responses or application errors.

## Frontend SDK Export

- Run `cargo xtask export-contracts` to regenerate the TypeScript DTOs, endpoint manifest, runtime-agnostic client, and browser `fetch` transport in `packages/contracts`.
- `Taskfile.yml` exposes the same workflow through `task export-contracts`, and `task test` refreshes the generated package before running the TypeScript and Rust test suites.
- The generated client builds URLs from contract-owned path metadata, serializes JSON request bodies after removing path parameters, and delegates execution to an injected transport.
- The generated browser transport resolves endpoint paths against a server `baseUrl` and decodes the shared web-server error envelope into a normalized SDK transport error.

## Project Slice Notes

- The current implementation keeps delete externally CRUD-shaped through `DeleteProjectHandler`.
- Repository implementations can still soft-delete internally by updating `is_deleted` and `updated_at`.
- `ora-db` now provides SQLite-backed implementations of the `ora-application` repository ports for `project`, `task`, `session`, and `worktree`.
- `ora-application` emits structured operational `tracing` events for project CRUD handlers with an `operation` field and, when available, a `project_id`. Success events log at `INFO`, and not-found or repository failures log at `ERROR` with failure details under `error`.
- The application layer emits events only; logging initialization, sink selection, and writer lifetimes stay owned by runtime composition roots such as `apps/web/server`.
