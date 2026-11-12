# Local Development

English | [中文](local-development.zh.md)

## Prerequisites

Install the Deno version pinned in `.deno-version`, Rust (see `rust-toolchain.toml`), Task, Git,
ripgrep and the Tauri system dependencies for your platform. Node.js and pnpm are not required.

```sh
task install:frontend   # locked npm dependencies through Deno, plus Git hooks
task run:desktop        # Desktop in development mode
```

`task --list` is the authoritative list of tasks.

## Toolchain pins

`.deno-version` is the Deno release used by CI and sidecar setup; keep `package.json` `engines.deno` in
sync (`deno task check:toolchain` checks both the installed runtime and the declaration, and runs before
installation, tooling checks and packaging). `DENO_VERSION` may only repeat the shared version.

The root `package.json` owns the workspace TypeScript version; every package uses its `tsc` and compiler
API and no child package pins its own compiler. `deno check` uses the compiler bundled with the pinned
Deno release. Third-party tools such as `ts-to-zod` keep the TypeScript 5 APIs they depend on.

## Running Desktop

`task run:desktop` builds the debug `ora-reaper` sidecar, runs `cargo xtask reconcile-migrations` against
the repository's `.data` directory (see [Persistence](../core/persistence.md#development-reconciliation))
and starts Tauri with `ORA_DATA_DIR` pointing at `.data`. Relative project locations in that database
resolve against the repository root. Debug builds use `deno` and `rg` from `PATH`.

- Logs are in `.data/logs/ora.log`; `task logdy` follows them in a local Logdy UI.
- Agents come from plugins, so install agent plugins from a marketplace source or import a local `.orax`
  before starting a chat.
- `task package:desktop` builds a complete local installer the way the release workflow does, with the
  release sidecars enabled.

## Controller and Node

The Controller and Node are Linux-only:

```sh
cargo build -p ora-controller -p ora-node -p ora-reaper
```

Their configuration is described in [Controller](../controller/controller.md) and [Node](../node/node.md).
The Cloud contract submodule is initialized with `task proto:init`, see
[Cloud contract](../controller/cloud-contract.md#obtaining-and-generating-the-contract). End-to-end runs
with Cloud, the Gateway and real sandboxes use the Compose stack in `ora-space/cluster`, which builds the
[container images](../operations/container-images.md).

## Maintenance scripts

- `deno run -A scripts/repair_workflow_snapshots.mjs --database <absolute-path-to-ora.sqlite3>` previews
  layout repairs of eligible workflow drafts; `--apply` applies them after a complete SQLite backup, see
  [Workflows](../workflows/workflows.md#loading-in-the-editor). `deno task test:tooling` runs its
  temporary-database regressions.
- `deno run -A scripts/workflow-qualification-suite/validate_sample.mjs` analyzes all generated
  qualification graphs through a running Desktop's `analyzeWorkflow` without creating workflows. The
  execution runner requires `ORA_QUALIFICATION_WORKSPACE_ID` to name an active, disposable isolated
  Workspace, because agents may edit files there. Reports state measured backend results and list
  unmeasured UI and memory checks explicitly; failures exit non-zero and reports stay local.
