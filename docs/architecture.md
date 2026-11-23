# Architecture

English | [中文](architecture.zh.md)

Ora is an IDE for AI agents. This repository builds two things from one Cargo workspace and one set of
TypeScript packages: the **Desktop application**, where a user works with agents on local projects, and
the **Controller and Node**, which carry Ora Cloud's work into sandboxes. This document gives the whole
picture; each part has its own document (see the [documentation map](README.md)).

## Desktop

```text
Web App Shell (packages/app-shell, React)
  │  @ora/contracts client  ─ Tauri IPC (commands + Channels)
  ▼
Desktop host (apps/desktop/src-tauri) ── surfaces: isolated webviews for UI plugins
  │  domain handles
  ▼
ora-backend ─ composition ─────────────────────────────────────────────┐
  ├─ ora-application  use cases ── ora-db  SQLite (ora.sqlite3)          │
  ├─ ora-agent-runtime  ACP sessions ──▶ agent plugins ──▶ agent CLIs    │
  ├─ plugin lifecycle  Deno plugin processes, marketplace, packs, Hooks  │
  ├─ workflow run engine, Effects, Git cleanup worker, scheduler         │
  └─ gitlancer  typed Git CLI ──▶ project checkouts and task worktrees ◀─┘
ora-reaper sidecar ── kills every registered process tree on exit or crash
```

- The frontend talks to the backend only through the generated `@ora/contracts` client; Rust owns every
  DTO and operation, see [Frontend contracts](core/frontend-contracts.md).
- `ora-backend` composes the domain crates behind narrow handles; the Tauri host stays thin, see
  [Application boundary](core/application-boundary.md) and [Desktop host](desktop/desktop-host.md).
- A **project** is a repository checkout (its Main Workspace); a **task** is an isolated Workspace
  backed by a linked worktree. Sessions and workflow runs execute in a Workspace, see
  [Tasks and worktrees](workspaces/tasks-and-worktrees.md).
- Every agent comes from an installed plugin that drives an agent CLI over ACP; Ora records each
  conversation itself so it can outlive or switch providers, see [Agent runtime](agents/agent-runtime.md)
  and [Session history](agents/session-history.md).
- Skills reach agents as files materialized into Workspaces by the Effect system; MCP servers reach them
  as session input, see [Effects](agents/effects.md) and [Session MCP](agents/session-mcp.md).
- Workflows orchestrate agent nodes over a frozen graph, see [Workflows](workflows/workflows.md).

## Cloud runtime

```text
Ora Cloud ◀── mTLS gRPC, dialed out ── ora-controller ── Substrate effects ──▶ Sandbox Server
                                            │                                     │
                                            └── WSS via sandbox router ──▶ sandbox: ora-node
                                                                              ├─ Git
                                                                              ├─ ora-reaper
                                                                              └─ agent plugin + CLI
```

- Cloud (the `ora-space/cloud` repository) is the authority for tenants, Workspaces, runs and
  permissions. The Controller claims Cloud's work, drives sandbox lifecycles and relays Node events and
  Thread records back, never deciding business state itself, see [Controller](controller/controller.md)
  and [Cloud contract](controller/cloud-contract.md).
- In each sandbox a Node clones the Workspace from a Git Server snapshot, installs plugins, runs Agent
  sessions with the same agent runtime Desktop uses, and delivers Revisions to the Git Server, see
  [Node](node/node.md).
- A Node runs every Git command as a direct child in its own process group, registered with an
  `ora-reaper` sidecar; a command with side effects holds an attempt lock whose release proves that it
  stopped, see [Node](node/node.md#execution-and-recovery).
- Authority is fenced end to end: Cloud authorizes, the Controller fetches a fresh permit per dispatch,
  and the Node isolates stale qualifications, see [Runtime control](controller/runtime-control.md).
- The Controller can also run a local SQLite form that coordinates clones for one Node.

## Workspace layout

| Path | Contents |
|---|---|
| `apps/` | Executables: `desktop/src-tauri`, `ora-controller`, `ora-node`, `ora-reaper` |
| `crates/` | Shared Rust crates, all prefixed `ora-` (domain, contracts, application, db, backend, agent-runtime, plugin-*, node-*, process, gitlancer, utils, …) |
| `packages/` | TypeScript packages: `app-shell`, `contracts`, `chat`, `editor`, `ui`, `plugin-sdk`, `workflow-runtime`, `workflow-mock` |
| `xtask/` | Contract export, migration reconciliation and the Rust size gate |
| `e2e/` | Desktop end-to-end tests with a fake ACP agent |
| `docker/` | Node and Controller images |
| `third_party/cloud` | The pinned Cloud contract submodule |
| `specs/` | An independent checkout of `ora-space/specs` (ADRs and core test cases), ignored by this repository |

Each crate and package keeps its internals in its own `README.md`; these documents cover what crosses
module boundaries.
