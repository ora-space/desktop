# Application and Contracts Boundary

The public application surface is split across `ora-application`, `ora-contracts`, `ora-backend`, and transport adapters so use-case orchestration and ACP streams are shared without coupling them to HTTP or Tauri.

## Ownership

- `ora-contracts` owns serialization-friendly request, response, and stream-event DTOs for Project, Task, Session, Skill, and Agent operations, including structured ACP prompts and provider-advertised commands, plus the Web-only project work context and filesystem operations.
- `ora-contracts::Project` is the single shared app-facing project payload for the first slice. It exposes `id`, `name`, and `root_path` only.
- `ora-contracts` keeps Rust field names idiomatic while serializing JSON payloads in `camelCase` for adapter and frontend consumption.
- `ora-contracts` also owns the frontend endpoint manifest for the exported HTTP CRUD surface, including operation names, HTTP methods, path templates, path parameters, request types, response types, and JSON body behavior.
- `ora-contracts` exports TypeScript DTOs plus the generated frontend SDK into `packages/contracts/src` so frontend packages can consume the generated contract surface from `@ora/contracts` and the browser transport from `@ora/contracts/fetch`.
- Backend-owned task worktrees stay internal; `ora-contracts` does not export standalone public worktree CRUD DTOs, SDK operations, or task payload linkage fields for them.
- `ora-application` owns CRUD handlers, application errors, repository ports, and domain-to-contract mapping.
- `ora-application` also owns the project work context handlers, lease timing rules, occupancy conflicts, and the mapping from `ora-domain::ProjectWorkContext` into the shared contract payload.
- `ora-backend` owns SQLite bootstrap, the system clock, concrete repository/handler composition, dynamic project selection for task Git operations, one application-scoped supervisor per supported agent CLI, grouped model discovery, per-session ACP routing, and transport-neutral public error normalization.
- Transport adapters stay thin: Web handlers and Tauri commands accept contract requests, delegate to the same `Backend`, then map its stable errors into HTTP or IPC semantics.
- `ProjectWorkContext` and filesystem browsing are deliberately outside `ora-backend` for now. The Web server keeps those existing services; Desktop reports `unsupported_operation` for those three contract operations.

## Frontend SDK Export

- Run `cargo xtask export-contracts` to regenerate the TypeScript DTOs and endpoint manifest in `packages/contracts`. The runtime-agnostic client and transports remain hand-written.
- `Taskfile.yml` exposes the same workflow through `task export-contracts`, and `task test` refreshes the generated package before running the TypeScript and Rust test suites.
- The generated client builds URLs from contract-owned path metadata, serializes JSON request bodies after removing path parameters, and delegates execution to an injected transport.
- Every transport request also retains the original complete request DTO. Fetch ignores that field, while the Tauri transport forwards it unchanged to the matching command.
- The generated browser transport resolves endpoint paths against a server `baseUrl` and decodes the shared web-server error envelope into a normalized SDK transport error.
- `ContractTransport.send` and `ContractTransport.stream` both accept an optional `AbortSignal`. Streams are cold and single-use. Web uses newline-delimited `data`, `error`, and `end` frames; Desktop uses the same private frame shape over a Tauri Channel.
- `apps/desktop/web` provides `createTauriTransport`; Desktop injects it through `createContractsClient(createTauriTransport())` and has no runtime dependency on the mock service.

## Project Slice Notes

- Project roots and Session task/provider routing are immutable after creation. Project updates rename only; Session lifecycle operations change only `status` and `updated_at`.
- Project and Task deletion use one SQLite transaction to soft-delete the complete Ora-owned aggregate. An active Session rejects the operation with `resource_in_use`; stopped children are cascaded. These deletion paths never call Git and never delete provider-owned ACP history.
- `ora-db` now provides SQLite-backed implementations of the `ora-application` repository ports for `project`, `task`, `session`, and `worktree`.
- `ora-application` emits structured operational `tracing` events for project CRUD handlers with an `operation` field and, when available, a `project_id`. Success events log at `INFO`, and not-found or repository failures log at `ERROR` with failure details under `error`.
- The application layer emits events only; logging initialization, sink selection, and writer lifetimes stay owned by runtime composition roots such as `apps/web/server`.
- `UpdateTaskRequest` cannot change project ownership. Task creation resolves the requested project's Git root. Deletion changes Ora database records only and deliberately leaves the linked Git worktree and branch untouched.
- Worktree paths are composed only when creating a new worktree. Existing worktree paths are never reconstructed from the configured creation root.
