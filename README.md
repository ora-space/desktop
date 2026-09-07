# Ora Desktop

Ora is an IDE for AI agents. The shared Rust crates live in `crates/`,
executable Rust packages live in `apps/`, and the shared TypeScript packages
live in `packages/`. All Rust packages share the root Cargo workspace.

## Architecture Docs

- [Task Workspace Files](docs/task-workspace-files.md) - read-only worktree
  browsing, bounded search, and native refresh

- [Application and Contracts Boundary](docs/application-contracts-boundary.md) —
  cross-crate ownership, contract shapes, and the handler set
- [Domain Models](docs/domain-models.md) — entities, identifier newtypes, and
  categorical enums
- [Frontend Contract SDK](docs/frontend-contract-sdk.md) — Rust-owned endpoint
  manifest, generation workflow, and transports
- [Gitlancer Architecture](docs/gitlancer-architecture.md) — typed Git CLI
  runtime
- [Workflow](docs/workflow.md) — definition management, draft/publish lifecycle,
  versioned snapshots, and run CRUD

## Runtime Docs

- [Desktop Runtime](docs/desktop-runtime.md) — Tauri commands, persistent paths,
  and configuration
- [ACP Agent Runtime](docs/agent-runtime.md) — provider supervision, session
  lifecycle, agent switching, and flow control
- [Session MCP](docs/session-mcp.md) — ACP `mcpServers` injection, live refresh,
  prompt admission, and the boundary that keeps MCP out of Workspace files
- [Session History](crates/history/README.md) — Ora's own conversation record
  and the handoff between agents
- [Runtime Logging](docs/runtime-logging.md) — configuration, JSON event
  contract, and Git command logging

## Persistence Docs

- [Database Migrations](docs/database-migrations.md) — migration catalog and
  reconciliation model
- [Database Repositories](docs/database-repositories.md) — SQLite adapters,
  pooling, and soft deletion
- [Task Worktrees](docs/task-worktrees.md) — workspace modes and backend-owned
  worktree lifecycle

## Development

Install the Deno version pinned in [`.deno-version`](.deno-version), Rust (see `rust-toolchain.toml`), Task, Git, ripgrep,
and the Tauri system dependencies for your platform. Node.js and pnpm are not
required. Run `task install:frontend` to install the locked npm dependencies
with Deno and configure Git hooks.

See [AGENTS.md](AGENTS.md) for code conventions. Common commands:

- `task test` — full lint and test suite for frontend and Rust workspace
  packages (long-running)
- `task lint` — all lint tasks
- `task export-contracts` — regenerate frontend contracts and plugin protocol bindings from Rust

## Toolchain versions

`.deno-version` is the Deno release pin consumed by CI and sidecar setup. Keep
`package.json`'s `engines.deno` in sync when changing it; `deno task check:toolchain`
checks both the installed runtime and this declaration. The check runs before
installation, tooling checks, and packaging. Sidecar setup verifies the content and native version of existing Deno and ripgrep
binaries before reuse; see [sidecar download verification](docs/sidecar-downloads.md). `DENO_VERSION` may
only repeat the shared version, not override it.

The root `package.json` owns the workspace TypeScript version. All packages use
its `tsc` and compiler API; child packages must not pin their own compiler.
Deno's `deno check` uses the compiler bundled with the pinned Deno release,
which currently matches the workspace compiler. Third-party tools such as
`ts-to-zod` still depend on TypeScript 5 APIs; those transitive dependencies
retain their supported versions instead of being forced across a major boundary.
