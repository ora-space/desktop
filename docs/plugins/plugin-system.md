# Plugin System

English | [中文](plugin-system.zh.md)

Plugins are how Ora is extended: agents, Skills, MCP servers, UI surfaces, Hooks, workflow
documents and packs of other plugins all arrive as installed `.orax` packages. This document covers
the package model, the process lifecycle, host services and configuration. Where packages come from
is in [Marketplace](marketplace.md); UI surfaces are in [Surfaces](surfaces.md); plugin logs are in
[Plugin logging](plugin-logging.md).

| Crate | Owns |
|---|---|
| `ora-plugin-manifest` | Parsing and validating `orax.toml` |
| `ora-plugin-manager` | Discovering installed packages, host-side package validation, install, update, local import |
| `ora-plugin-lifecycle` | The only owner of plugin processes; host request services; runtime state |
| `ora-plugin-runtime` | Launching Deno, the stdio protocol, registration, calls and notifications |
| `ora-plugin-protocol` | The wire contract shared with `packages/plugin-sdk` |
| `ora-plugin-config` | Setting declarations, MCP and Hook configuration shapes, stored values |
| `ora-plugin-registry` | Marketplace source sync and the available-plugin index |
| `ora-backend` (`src/plugin`) | Composition: operations, packs, Hook lifecycle, marketplace sync |

## Identity and layout

A plugin is identified by `ora_domain::PluginId`, spelled `<namespace>/<name>` on every contract.
The manifest never names its namespace: the host derives it from the marketplace source the package
was installed from (or `local` for an imported archive), because the namespace decides which install
directory, data directory and Skill rows a package owns.

```text
~/.ora/plugins/
  installed/<namespace>/<name>/<version>/   read-only package (orax.toml, main.js, assets/, logo.svg)
  data/<namespace>/<name>/                  plugin data: store.json, ora/storage root, downloads, web profile
  logs/<namespace>/<name>/                  host-managed plugin log, see Plugin logging
  sources/                                  marketplace source checkouts
  cache/                                    registry_index.json and in-flight .orax downloads
  log-levels.json                           per-plugin log levels
```

Discovery scans `installed/`, parses each version directory as SemVer and selects only the highest
version per plugin, with no fallback when that version is invalid. Namespace and identifier come from
the directory levels and must agree with the manifest; malformed packages become structured
discovery issues and stay out of the snapshot. Only an explicit scan rebuilds the snapshot.

## Kinds

| Kind | Contributes | Process |
|---|---|---|
| `agent` | Exactly one agent, identified by the package's plugin id, see [Agent runtime](../agents/agent-runtime.md) | Deno `main.js`, broad permissions, may spawn host-managed child processes |
| `workbench` | A page shipped in the package and shown as a surface | Deno `main.js` with no `--allow-*` permission, started by the page's first bridge call |
| `webview` | An external HTTPS site shown as a surface | None |
| `skill` | One or more Skills under `assets/<name>/SKILL.md` | None |
| `mcp` | One MCP server from `assets/config.json`, see [Session MCP](../agents/session-mcp.md) | None (the agent runs the server) |
| `hook` | A native executable with `init`/`deinit` lifecycle commands | None (the host runs commands only when authorized) |
| `workflow` | Workflow documents imported into the library, see [Workflow packages](workflow-packages.md) | None |
| `pack` | A list of other plugins from the same source, see [Marketplace](marketplace.md#packs) | None |

The manifest crate checks shape (field types, the identifier grammar, that `[webview]` exists
exactly for webview plugins and `[workbench]` only for workbench plugins) and ignores unknown fields
so additive schema changes do not remove listings from older clients; breaking changes are gated by
`resolver`. The manager adds what depends on the package on disk: `main.js` exists exactly for
agent and workbench packages, a workbench ships `assets/index.html`, a webview's `start_url` is
covered by `allowed_origins`, an MCP stdio command resolves to a regular file inside the package,
and a Hook executable resolves to a regular non-symlink file under `assets/`.

A release is either one universal `url` + `sha256` or a set of `[[targets]]`, each with an exact
Rust target triple. Targeted releases are allowed only for kinds that ship a native binary: `hook`
(which is that binary) and `agent` (which may bundle its CLI). An installed targeted package carries
an `[artifact]` section with its target, checked against the host on both online install and local
import; the target is never part of the identity. Ora no longer runs on macOS (v0.6.0 was the last
macOS release), so the `*-apple-darwin` triples are retired: a listing's darwin entries are skipped
instead of failing the listing, a listing with only darwin entries has no installable release, and
an installed `[artifact]` naming a darwin triple is refused.

## Lifecycle

`ora-plugin-lifecycle` is the sole owner of plugin processes, so the runtime state Settings shows is
always the set of processes that exist. Every installed plugin is available; agent and workbench
processes start on demand and can be stopped without changing availability, while the other kinds
have no process and stay `stopped`. All plugins start stopped when the lifecycle opens.

- Consumers that speak a protocol connect instead of launching: `ensure_running` activates a stopped
  or failed plugin and waits, bounded, and `connection` returns a `PluginConnection` pinned to one
  generation, so a restarted plugin is never addressed through a stale one.
- Each launch runs an exit monitor and a notification pump. If notifications stop while the process
  is still alive past a short grace, the attempt is marked failed.
- Per-plugin operations serialize on that plugin's lock; unrelated plugins progress independently.
  A scan takes all locks in stable order to return one coherent snapshot.
- Uninstall closes the plugin's surfaces, stops its process, then stages
  `installed/<namespace>/<name>` (and, when requested, its data and log trees) before committing the
  removal; a staging failure rolls the moves back. See [Plugin logging](plugin-logging.md#lifecycle)
  for the delete-data case.

### Launch

The lifecycle creates `data/<namespace>/<name>/` (with `downloads/`), derives Deno permissions from
the kind, and starts `main.js` with the package root as cwd. No environment variable is injected; a
plugin learns no host path. After the handshake the registration is validated against the kind:
workbench plugins may expose methods but cannot declare emitted notifications, and agent contracts
are verified by the agent runtime.

## Protocol and SDK

`crates/plugin-protocol` owns the wire contract shared by the Rust host, test plugins and
`packages/plugin-sdk`: framing, JSON-RPC method constants, registration and DTOs for agent control,
Effects, storage, child processes and workbench calls. `task export-contracts` writes generated
bindings into `packages/plugin-sdk/src/protocol` (`constants.ts` and DTO modules); `index.ts`,
`json.ts` and `transport.ts` stay hand-written. Never duplicate a wire name in the SDK: add it to
`ora-plugin-protocol` and regenerate.

Stdout carries only protocol frames: a four-byte big-endian length (including the type byte), a
one-byte frame type and a UTF-8 JSON payload, at most 16 MiB. A protocol failure invalidates the
process. Multi-word path segments are snake_case (`agent/list_models`, `effect/verify_ready`,
`ora/childprocess/close_stdin`). Stderr carries diagnostics, see [Plugin logging](plugin-logging.md).

A plugin declares both traffic directions once, in `ora/register`:

```jsonc
{
  "method": "ora/register",
  "params": {
    "methods": ["agent/start", "effect/coordinate", "effect/reactivate", "effect/verify_ready"],
    "emits": ["agent/acp"],
    "effectResources": [{
      "workspaceRelativePath": ".agents/skills",
      "materializationFormat": "ora/skill-directory.v1",
      "coordination": "quiesce_before_mutation"
    }]
  }
}
```

The host can invoke only listed `methods`; a notification outside `emits` invalidates the
connection. `effectResources` declares the Workspace Resources an agent consumes, see
[Effects](../agents/effects.md). Requests from the plugin to the host need no declaration.

`agent/list_models` takes `AgentListModelsParams { cwd }`, the Workspace the discovery is for; the
host calls it on demand with a longer budget than ordinary control calls.

### Host services

The launch binds one request handler to the plugin, so its identity comes from the launch, never
from request parameters.

- **Storage** (`ora/storage/list`, `read`, `write`, `remove`) works on logical paths relative to the
  plugin's data directory. Paths are `PortableRelativePath`s resolved without traversing symlinks;
  the host-owned `web-profile/` subtree is refused. Files are capped at 8 MiB because base64 must fit
  in a frame. Errors carry `data.kind` (`invalid_params`, `invalid_path`, `not_found`, `too_large`,
  `io`).
- **Child processes** (`ora/childprocess/spawn`, `write`, `close_stdin`, `kill`, agent plugins only)
  let a plugin have the host own a subprocess with tree-wide termination. `spawn` takes either
  `command` (resolved by the OS) or `packageCommand` (joined onto the plugin's install root and
  required to stay inside it). `package_command_missing` means the package carries nothing at that
  path, the one failure a plugin should answer by falling back to `PATH`; the SDK's
  `spawnAgentProcess` implements that. The host pushes `ora/childprocess/stdout`, `stderr` and `exit`
  notifications, never sending exit before the last output chunk, and kills every child of a
  generation when that generation exits.

## Operations

| Operation | Purpose |
|---|---|
| `listInstalledPlugins`, `scanPlugins` | The installed snapshot; an explicit rescan |
| `installPlugin`, `updatePlugin`, `importPlugin` | Install or update from the marketplace; import a local `.orax` into the `local` namespace |
| `activatePlugin`, `stopPlugin`, `uninstallPlugin` | Process control and removal (optionally deleting data) |
| `getPluginConfiguration`, `savePluginConfiguration`, `resetPluginConfiguration` | Plugin settings, see below |
| `listHookLifecycleReports`, `initializeHook` | Hook lifecycle results; a user-requested `init` |
| `getPluginLogLevel`, `setPluginLogLevel` | See [Plugin logging](plugin-logging.md) |
| `listMcpHealth`, `probeMcpHealth` | See [Session MCP](../agents/session-mcp.md#host-mcp-health) |

Installation downloads the `.orax` (verifying SHA-256 while downloading), extracts it safely into a
staging directory and commits it with one rename into `installed/<namespace>/<name>/<version>`. The
package is validated from the manifest inside the archive, so an installed package is discoverable
by construction; its `identifier`, `version` and `kind` must match the listing, while descriptive
metadata may differ and the package wins. An update refuses no-ops and downgrades before
downloading, then retires every other version. Install, update and uninstall reconcile the agent set
and emit `PluginStatusChanged`.

## Configuration

A package may declare configuration in `assets/config.json`, which has four mutually exclusive
shapes: version-one settings only; an MCP configuration (settings plus exactly one `stdio` or `http`
transport); a Hook configuration (settings plus a `hook` descriptor); or a version-two Agent
declaration (`agent.modelAccess` and optional Settings). Mixing contribution keys fails closed.
MCP and Hook packages require their respective shapes; only an Agent package may carry the Agent shape.

Stored values for the version-one shapes are plugin-global, independent of installed versions, and live in
`data/<namespace>/<name>/store.json`. Saves replace the complete override set atomically and use an
optimistic revision plus a declaration fingerprint, so a stale editor cannot overwrite newer state.
A malformed store is kept under a backup name on recovery. A package with an invalid declaration
stays visible but cannot start. List responses include each plugin's configuration completeness.
MCP settings are limited to `string`, `number` and `boolean`; HTTP headers must bind through setting
references, so credentials cannot be baked into a package.

The [Agent configuration declaration](agent-configuration.md) records static model-access
capabilities and optional Settings with an exact-byte identity. Release listings and Node install
plans may bind that identity to a package; the installer checks the package's declaration before
committing. The Agent Settings subset feeds the local settings store and editor under the
declaration's raw-byte fingerprint, while the model-access half has no local execution semantics;
neither compiling nor installing a declaration enables a personal model configuration or authorizes execution.

## Hooks

A Hook package ships a program that manages its own agent integration; the host never performs that
integration itself. The host owns only the boundary around running it:

- Installing and running are separate authorizations. `init` runs after a single-plugin install,
  update or import, and `deinit` before an uninstall stages the package away, only when the request
  carries `hookExecutionAcknowledged`. A pack install runs nothing for its members. `initializeHook`
  runs `init` again on explicit request.
- The executable is resolved inside the installed package and re-validated before every spawn. It
  runs without a shell, in `~/.ora` as working directory, with a minimal environment and no setting
  values, a 120-second limit and bounded output (256 KiB captured, 4 KiB kept in the report).
- A failed command never changes installation state. Results are kept for the current session only
  (`listHookLifecycleReports`), because the durable truth is the agent configuration the tool itself
  wrote.

## Decisions

- [Versioned dual-form manifest](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/manifest/0-versioned-dual-form-manifest.md)
- [Static MCP configuration](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/category/mcp/0-static-mcp-configuration.md)
- [Hook plugins](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/category/hook/0-hook-plugin-pack.md)
- Other plugin categories: [`decisions/desktop/plugin/category/`](https://github.com/ora-space/specs/tree/main/decisions/desktop/plugin/category)
