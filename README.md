# Ora Desktop

Ora is an IDE for AI agents. The Rust backend lives in `crates/`, the two runtime roots in `apps/web/server` and `apps/desktop`, and the shared TypeScript packages in `packages/`.

## Architecture Docs

- [Task Workspace Files](docs/task-workspace-files.md) - read-only worktree browsing, bounded search, and native refresh

- [Application and Contracts Boundary](docs/application-contracts.md) — cross-crate ownership, contract shapes, and the handler set
- [Domain Models](docs/domain-models.md) — entities, identifier newtypes, and categorical enums
- [Frontend Contract SDK](docs/frontend-contract-sdk.md) — Rust-owned endpoint manifest, generation workflow, and transports
- [Gitlancer Architecture](docs/gitlancer-architecture.md) — typed Git CLI runtime
- [Workflow](docs/workflow.md) — definition management, draft/publish lifecycle, versioned snapshots, and run CRUD

## Runtime Docs

- [Web Server Runtime](docs/web-server-runtime.md) — configuration, HTTP API, and error semantics
- [Desktop Runtime](docs/desktop-runtime.md) — Tauri commands, persistent paths, and configuration
- [ACP Agent Runtime](docs/agent-runtime.md) — provider supervision, session lifecycle, agent switching, and flow control
- [Session History](crates/history/README.md) — Ora's own conversation record and the handoff between agents
- [Runtime Logging](docs/runtime-logging.md) — configuration, JSON event contract, and Git command logging

## Persistence Docs

- [Database Migrations](docs/database-migrations.md) — migration catalog and reconciliation model
- [Database Repositories](docs/database-repositories.md) — SQLite adapters, pooling, and soft deletion
- [Task Worktrees](docs/task-worktrees.md) — workspace modes and backend-owned worktree lifecycle
- [Project Work Contexts](docs/project-work-contexts.md) — window-to-project leases and occupancy rules

## Development

See [AGENTS.md](AGENTS.md) for code conventions. Common commands:

- `task test` — full lint and test suite for frontend, backend, and Desktop (long-running)
- `task lint` — all lint tasks
- `task export-contracts` — regenerate the TypeScript contract package from Rust
