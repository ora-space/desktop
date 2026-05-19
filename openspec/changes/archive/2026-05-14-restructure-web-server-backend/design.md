## Context

`apps/web/server` currently contains all bootstrap code in `src/main.rs`. That file mixes environment-driven logging setup, in-memory repository wiring, application handler construction, and the beginnings of transport composition, but it never starts a network server or exposes HTTP routes. The current crate layout also leaves no clear place to add runtime configuration, shared server state, route registration, request handlers, or transport-level error mapping without continuing to grow a single file.

The surrounding workspace already provides the pieces a backend adapter should compose: `ora-application` exposes transport-agnostic project handlers, `ora-contracts` defines serialization-friendly DTOs, and `ora-logging` provides shared bootstrap logging. This change should turn `ora-web-server` into a complete first backend runtime while preserving those existing boundaries.

## Goals / Non-Goals

**Goals:**

- Split `apps/web/server/src/main.rs` into focused modules with a clear composition root.
- Add a real HTTP server runtime that starts successfully after logging initialization.
- Expose HTTP endpoints for the existing project CRUD slice using `ora-application` handlers and `ora-contracts` DTOs.
- Define shared server state and bootstrap configuration so the adapter can grow without re-centralizing everything in `main.rs`.
- Keep the first runtime slice easy to test locally by using an in-memory repository implementation behind adapter-owned wiring.

**Non-Goals:**

- Replacing the bootstrap repository with SQLite-backed persistence in this change.
- Adding HTTP endpoints for `task`, `worktree`, or `session` before their server-facing workflows are needed.
- Designing authentication, authorization, CORS policy hardening, or production deployment manifests.
- Changing `ora-application`, `ora-contracts`, or domain behavior beyond the adapter-facing needs required to run the server.

## Decisions

### Introduce a module-oriented server crate layout

`apps/web/server/src` will move from a single-file entry point to a small module tree built around one thin `main.rs`. The expected layout is:

- `main.rs`: process entry point that initializes logging, loads config, builds the app, and starts the server
- `config.rs`: environment-backed server configuration
- `bootstrap.rs`: composition root for repositories, clocks, ID generators, and application handlers
- `app_state.rs`: shared runtime state exposed to transport handlers
- `routes.rs`: top-level router construction
- `handlers/`: HTTP handler functions grouped by concern such as `health` and `projects`
- `error.rs`: transport error mapping from application failures to HTTP responses

Why:
- This keeps each concern local and prevents `main.rs` from becoming the permanent dumping ground for server logic.
- It matches the repository guidance to prefer smaller modules and creates natural seams for incremental tests.

Alternative considered:
- Keep one file and use internal sections only.
  Rejected because the user explicitly wants the file split and the server crate needs durable structure, not just shorter scrolling distance.

### Use a conventional Rust HTTP stack in the server crate

`ora-web-server` will add a standard async HTTP stack suitable for a small backend adapter: an HTTP framework for routing and extraction, a Tokio runtime, and serde-based JSON support. The server will bind to a configured socket address and expose JSON endpoints for the project CRUD surface plus lightweight health endpoints.

Why:
- The crate currently has no runtime or transport library, so becoming a usable backend server requires adding one.
- A conventional ecosystem choice reduces custom infrastructure and makes the adapter easier for future contributors to extend.

Alternative considered:
- Implement a custom TCP or minimal hyper-only stack.
  Rejected because it adds unnecessary boilerplate for routing, extraction, and JSON response handling.

### Keep composition transport-focused by wrapping existing project handlers in shared state

The server adapter will continue to use `CreateProjectHandler`, `GetProjectHandler`, `ListProjectsHandler`, `UpdateProjectHandler`, and `DeleteProjectHandler` as the use-case boundary. `bootstrap.rs` will construct one `WebProjectApi`-style aggregate or equivalent shared state object, and request handlers will delegate to that state instead of performing business orchestration inline.

Why:
- This preserves the existing architecture where transport adapters translate requests and responses but do not own use-case logic.
- It makes it straightforward to replace the in-memory repository with a database-backed implementation later without rewriting route handlers.

Alternative considered:
- Call repositories directly from HTTP handlers for the first slice.
  Rejected because it would bypass `ora-application` and erode the layered contract already established in the workspace.

### Add explicit server configuration instead of embedding defaults in bootstrap logic

The adapter will define a typed server configuration that reads `ORA_HOST` and `ORA_PORT`, alongside the existing logging configuration. The default bind address will be `0.0.0.0:32578` so local development and container-style execution share one predictable startup contract, and invalid configuration values will fail during bootstrap with typed errors instead of late runtime surprises.

Why:
- A complete backend server needs a stable bind contract, not hard-coded addresses hidden in `main.rs`.
- Typed configuration errors fit the existing bootstrap error pattern used for logging initialization.

Alternative considered:
- Hard-code the listener address and defer configuration until later.
  Rejected because server startup behavior is part of the runtime contract and should be explicit from the first implementation.

### Make readiness depend on successful application-state bootstrap

The readiness endpoint will report success only after the server finishes constructing its application state successfully. Liveness remains a lightweight process-level check, while readiness confirms the runtime is actually prepared to serve use-case requests with initialized bootstrap dependencies.

Why:
- This gives callers a more reliable signal than bare process startup, especially once the composition root grows beyond the in-memory bootstrap implementation.
- It matches the user's intended semantics for readiness without coupling liveness to application wiring.

Alternative considered:
- Make readiness identical to liveness and report success as soon as the process starts.
  Rejected because it would not distinguish between a running process and a usable backend runtime.

### Ship project CRUD over HTTP before broader resource coverage

The first complete backend runtime will expose only the already-supported project CRUD flows plus simple health/readiness endpoints. The crate structure will leave clear room to add `task`, `worktree`, and `session` routes later without revisiting the composition root.

Why:
- The project CRUD slice is already implemented in `ora-application` and is the fastest path to a real backend server.
- This keeps the change bounded while still delivering an end-to-end useful service.

Alternative considered:
- Wait until every entity has HTTP routes before calling the crate a backend server.
  Rejected because the user asked for a complete server structure now, and a modular first slice is more valuable than postponing all runtime work.

## Risks / Trade-offs

- [Adding an async HTTP stack increases dependency surface] -> Mitigation: keep the dependency set limited to the runtime, router, and JSON serialization pieces required for the first server slice.
- [An in-memory repository means data is ephemeral across restarts] -> Mitigation: keep that limitation explicit in docs and composition naming so callers understand this is a bootstrap runtime shape, not final persistence.
- [Transport error mapping can drift from application error semantics] -> Mitigation: centralize mapping in one adapter error module and cover representative success and failure responses in tests.
- [Modularization can still leave ambiguous ownership if boundaries are fuzzy] -> Mitigation: keep `bootstrap`, `routes`, `handlers`, and `config` responsibilities narrow and avoid cross-calling between handler modules outside shared state.

## Migration Plan

1. Add the HTTP runtime dependencies to `apps/web/server`.
2. Extract existing bootstrap pieces out of `main.rs` into `config`, `bootstrap`, and supporting modules without changing behavior.
3. Introduce router construction, app state, and HTTP handler modules.
4. Start the server after logging initialization and expose health plus project CRUD endpoints.
5. Add focused tests for configuration parsing, transport error mapping, and representative route behavior.

Rollback strategy:
- Revert the `ora-web-server` crate changes as one unit if the runtime fails to stabilize; no schema or persisted data migration is involved in this change.

## Open Questions

- None at this stage.
