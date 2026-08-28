# Agent MCP-Config Wiring — Research

> Investigate how an installed MCP plugin's configuration reaches an Agent plugin's
> process so the agent can use the MCP during conversation, and define the minimum
> closed loop: install `ora-space.opencode` (agent) + `ora-space.tavily-search` (mcp)
> from the `ora-space/marketplace`, then get tavily's MCP config into the opencode
> agent's working directory, mirroring how Ora already wires **skills**.
>
> Date: 2026-08-27. Primary sources only: Ora Rust crates, the `specs/` nested repo,
> the `ora-space/marketplace` + `ora-space/opencode-agent` + `ora-space/tavily-search-mcp`
> repos, and `opencode.ai/docs`. Every claim carries a citation.

---

## 0. Executive summary

- **Supply side is fully built.** Download → sha256 → extract → kind-aware validate →
  commit → discover is implemented in `ora-plugin-manager`. `assets/config.json` compiles
  to a `CompiledMcpConfiguration` in `ora-plugin-config`; user setting values persist in
  a revisioned `store.json`. Verified end-to-end for both target plugins.
- **Demand side is entirely absent.** `ResolvedMcp` — the concrete transport with real
  URLs, resolved setting values, and a resolved `workspace` cwd — **does not exist as a
  type anywhere in the codebase** (grep for `ResolvedMcp|resolve_mcp` returns only three
  doc comments, no definition). Nothing combines the compiled config with `store.json`
  and pushes it to an agent.
- **No delivery channel is wired.** Three candidate channels exist on paper; **none is
  implemented**: (A) an Effect filesystem surface with an `mcp_config.v1` format — only
  `skill_directory_v1` exists and the validator rejects the rest
  (`agent_runtime/plugin_agent/effect.rs:67`); (B) the ACP `session/new` `mcp_servers`
  field — present in the schema but Ora always sends it empty
  (`agent_runtime/warm.rs:526`); (C) the spec's `ora/agent/configure_agent` method —
  **zero references** in `crates/`.
- **The opencode agent plugin v0.2.2 declares only a skills surface**
  (`.opencode/skills`, `skill_directory.v1`). It has no MCP-config surface and no code
  that renders an MCP descriptor into `opencode.json`. The tavily README states this step
  is the agent plugin's job ("an Agent plugin later turns the installed descriptor into
  target-agent configuration") — but the opencode plugin hasn't implemented it.
- **Spec vs code divergence.** The active spec assigns MCP rendering to the agent plugin
  via `configure_agent` (`specs/active/plugin/5-mcp.md#agent-配置时序`). The implemented
  skills path uses a different mechanism — the Effect filesystem surface worker — which
  the spec only proposes extending to MCP in `changes/effect/v2.md` (`mcp_config.v1`).
- **Recommendation:** build `ResolvedMcp`, then deliver via the **Effect filesystem
  surface (Channel A)** — it is the literal "mirror skills" path, reuses the proven
  `EffectWorker` + `FilesystemSurfaceAdapter`, and is the channel the
  `AGENTS.md` convergence rule already anticipates. See §7–§8.

---

## 1. The minimum closed loop — what works today vs. what is missing

Goal: install both plugins, save a tavily API key, then in an opencode conversation use
tavily web-search MCP.

| Step                                                                  | Status                                | Owner                                  |
| --------------------------------------------------------------------- | ------------------------------------- | -------------------------------------- |
| Browse marketplace, pick opencode + tavily                            | ✅                                    | marketplace registry                   |
| Download `.orax`, verify sha256, extract, validate, commit            | ✅                                    | `plugin-manager/install.rs`            |
| Compile tavily `assets/config.json` → `CompiledMcpConfiguration`      | ✅                                    | `plugin-config/src/mcp/mod.rs`         |
| Persist tavily `apiKey` in `store.json` (settings UI)                 | ✅                                    | `plugin-config/src/service.rs`         |
| Discover opencode agent plugin, spawn Deno process, `agent/start`     | ✅                                    | `plugin-lifecycle`, `backend`          |
| opencode plugin spawns `opencode acp --cwd <cwd>`                     | ✅                                    | opencode plugin `lifecycle.ts`         |
| **Resolve tavily descriptor + `store.json` → concrete HTTP endpoint** | ❌                                    | **gap: `ResolvedMcp`**                 |
| **Select which MCPs are enabled for the agent's Workspace**           | ❌                                    | **gap: per-Workspace MCP desired set** |
| **Deliver resolved MCP config to the opencode agent**                 | ❌                                    | **gap: no channel wired**              |
| opencode CLI loads tavily from `opencode.json` `mcp` block            | ⚠️ target format known, writer absent | opencode plugin / Ora                  |

The three ❌ rows are the work. Everything above them ships today.

---

## 2. Supply side (installed) — install → compile → store

### 2.1 Install flow

`Installer::install` (`crates/plugin-manager/src/install.rs:144`) orchestrates download →
extract → validate → commit. Download + sha256 in `download_package` (`install.rs:344-373`);
the manifest's `sha256` becomes a `Checksum::sha256` on the `DownloadRequest`, verified by
the `HttpDownload` impl (`crates/utils/src/http/reqwest.rs:190`,
`crates/utils/src/http/local.rs:111`). Local-import path `Installer::install_local`
(`install.rs:254-341`) re-verifies the digest against the archive (`install.rs:296-305`).
Extraction is `ora_utils::archive::extract_archive` with `ArchiveFormat::Zip`
(`install.rs:174-179`). Validated staging tree is atomically renamed to
`<data-dir>/plugins/installed/<namespace>/<name>/<version>/` (`install.rs:186-189`).

### 2.2 Kind-aware validation (where `kind=mcp` policy lives)

`validate` (`crates/plugin-manager/src/validation.rs:116-215`) enforces two MCP rules:

- Cross-kind exclusion (`validation.rs:145-158`): a non-MCP package shipping a
  transport-bearing config file is rejected — "only `mcp` packages may declare an MCP
  transport".
- MCP-specific (`validation.rs:180-182`) delegates to `validate_mcp`
  (`crates/plugin-manager/src/mcp.rs:29-69`): must not ship `main.js` (`mcp.rs:35-39`,
  config-only); config must compile to the MCP shape, not Settings-only
  (`mcp.rs:56-60`); stdio command must be a real executable contained inside the package
  (`mcp.rs:64-66`, `validate_command_containment` `mcp.rs:73-128`). Produces
  `InstalledMcpDescriptor { configuration: CompiledMcpConfiguration }` (`mcp.rs:20-22`).

### 2.3 The value model (install-time, immutable)

`crates/plugin-config/src/mcp/mod.rs`:

```rust
// mod.rs:42-50
pub struct CompiledMcpConfiguration {
    pub schema_version: u32,                     // always 1
    pub settings: Option<CompiledDeclaration>,   // user-facing Settings subset
    pub transport: McpTransport,                 // exactly one transport
}
// mod.rs:55-58
pub enum McpTransport { Stdio(McpStdioTransport), Http(McpHttpTransport) }
// mod.rs:62-68
pub struct McpStdioTransport { command: PortableRelativePath, args: Vec<McpArgument>, env: BTreeMap<String, McpValueExpression> }
// mod.rs:72-75
pub struct McpHttpTransport { url: Url, headers: BTreeMap<String, McpValueExpression> }
// mod.rs:79-83
pub enum McpArgument { Value(McpValueExpression), WorkspaceContext }   // { "context":"workspace" } → agent cwd
// mod.rs:90-97
pub enum McpValueExpression { Literal(String), Setting { id: String, prefix: String, suffix: String } }
```

`compile_configuration_file` (`mod.rs:130-149`) dispatches by the `transport` member.
`McpValueExpression::Setting` is validated at compile time — the referenced `setting` id
must exist in declared settings (`transport.rs:241-262`) — but is **never substituted at
runtime** (see §4).

### 2.4 Persistence

`store.json` at `<data-dir>/plugins/data/<namespace>/<name>/store.json`
(`crates/plugin-config/src/service.rs:487-499`), so tavily maps to
`plugins/data/official/ora-space.tavily-search/store.json` (dotted name segments preserved,
`service.rs:696-749`). Shape:

```rust
// crates/plugin-config/src/values.rs:9-15
struct StoredConfiguration { schema_version: u32, revision: u64, values: BTreeMap<String, SettingValue> }
```

Optimistic revision (`service.rs:263-272`), declaration fingerprint guard
(`declaration.rs:21,126-127`; checked `service.rs:251-253`), atomic write with
platform permission tightening (`filesystem.rs:59-73`), corrupt-file backup
(`service.rs:300-371`). The editor-side `ConfigurationService::get` + `details_from`
merges declaration + store for the settings UI (`service.rs:196-215`, `values.rs:28-82`)
— but produces nothing an agent process can consume.

---

## 3. Agent runtime — two `cwd` values

Ora distinguishes two working directories:

- **Process `cwd` at `agent/start` = `home_directory` (neutral, NOT the workspace).**
  `AgentStartParams { cwd, host_version }` (`crates/backend/src/agent_runtime/plugin_agent/control.rs:58-63`)
  sent by `start_agent` (`control.rs:155-161`). `home_directory` from `BackendPaths`
  (`bootstrap.rs:42`) flows through `ConnectionSupervisors` → `SupervisorContext`
  (`connection.rs:122,155`) into `spawn_initialized_process` (`connection.rs:573`).
- **Session `cwd` at `session/new` = the Workspace directory (project or worktree path).**
  `warm.rs:513-527` calls `session/new` with `NewSessionRequest::new(cwd)`; the `cwd` is
  resolved via `resolve_warm_cwd` → `workspace_cwd` → `resolve_workspace_cwd`
  (`agent_runtime/mod.rs:312,853,874-878`). Confirmed by `agent_runtime/README.md:18`.

The agent process itself is spawned and owned by the **plugin**, not Ora
(`plugin_agent/README.md:32`: "A plugin spawns and owns its agent CLI itself"). Ora passes
the workspace path via `session/new`.

---

## 4. Skills materialization — the mechanism to mirror

This is the reference pattern. Skills reach the agent's workspace as **files at a path the
agent plugin declares**, written by Ora's Effect worker.

### 4.1 The pipeline

1. **Trigger** — `SqliteEffectRepository::publish_source`
   (`crates/db/src/repository/effect/mod.rs:45-177`) at startup
   (`skill_reconciliation.rs:30-119` line 82), on plugin discovery
   (`PluginApi::sync_installed_skills` `crates/backend/src/plugin.rs:243-251`), and on user
   create/update.
2. **Desired set** — `install_source_in_all_workspaces`
   (`db/src/repository/effect/mapping.rs:207-227`) inserts the source into every
   Workspace's `workspace_effect_desired_items` (`effect/mod.rs:161-168`); updates call
   `upsert_propagation_request` (`effect/mod.rs:172`).
3. **Surface declaration (by the agent plugin)** — the opencode plugin declares
   `effectSurfaces` during registration; `registered_skill_surfaces`
   (`crates/backend/src/agent_runtime/plugin_agent/effect.rs:54-89`) converts them to
   `FilesystemSkillSurface` (rejecting any format ≠ `skill_directory_v1`, `effect.rs:67-69`).
   Persisted by `PluginApi::replace_agent_effect_surfaces` (`plugin.rs:465-507`) which
   wakes the worker (`plugin.rs:503-505`).
4. **Convergence** — `EffectWorker::converge_surface_registrations`
   (`effect_worker.rs:278-307`) reads `agent_effect_surface_declarations()`
   (`plugin.rs:514-522`) and, for Workspaces missing the surface, calls
   `converge_workspace_surfaces` (`effect_surface_registration.rs:22-58`) which writes
   surfaces for every Workspace in the same pass — the "new Workspace → existing consumer"
   direction the `AGENTS.md` rule demands.
5. **Claim + reconcile** — `run_pass` (`effect_worker.rs:220-243`) claims due
   `effect_reconcile_requests` and calls `reconcile_one` (`effect_worker.rs:461`),
   constructing a `FilesystemSurfaceAdapter` (`effect_worker.rs:467-472`).
6. **Filesystem write** — `FilesystemSurfaceAdapter::stage`
   (`crates/effect/src/filesystem.rs:268-301`) copies the skill package, writes a
   `.ora-managed.json` ownership marker; `apply_create` (`filesystem.rs:314-327`) atomically
   renames staged → `<workspace_root>/<surface_path>/<skill_name>/`. The surface root is
   `workspace_root.join(surface_path)` (`filesystem.rs:120-121`).
7. **Consumer coordination** — before write, `quiesce` calls `effect/waitForIdle`
   (`effect_worker.rs:621-646`); after, `resume` calls `effect/restart`
   (`effect_worker.rs:656-680`) with `{surfaceKey, workspaceRoot, relativePath}`
   (`agent_runtime/plugin_agent/effect.rs:119-134`).

### 4.2 The on-disk result

Skills materialize at `<workspace_root>/.opencode/skills/<skill_name>/SKILL.md` +
`.ora-managed.json`, confirmed by `effect_worker.rs:986-994`. The `.opencode/skills` path is
**plugin-declared** by the opencode plugin (`effect_worker.rs:869`,
`effect_surface_registration.rs:113-115`):

```rust
FilesystemSkillSurface {
    workspace_relative_path: SurfacePath::parse(".opencode/skills").unwrap(),
    materialization_format: MaterializationFormat::skill_directory_v1(),
    consumer: ConsumerId::new("official/ora-space.opencode"),
    coordination: ConsumerCoordination::WaitForIdleAndRestart,
}
```

### 4.3 Key invariant

The write is **asynchronous convergence**, driven by durable SQLite reconcile requests +
a 30s safety scan (`SCAN_INTERVAL` `effect_worker.rs:32`), level-triggered
(`desired_generation > ready_generation`). It is NOT at `agent/start` or `session/new`.

---

## 5. MCP demand side — the gap

### 5.1 `ResolvedMcp` does not exist

Grep for `ResolvedMcp|resolve_mcp|McpResolved` across all `.rs` returns **zero type
definitions**. Three doc comments name it as future work:

- `crates/plugin-config/src/mcp/mod.rs:6-7` — "Resolution against `store.json`
  (`ResolvedMcp`) is a later, separate step and is deliberately not modeled here."
- `crates/plugin-config/src/mcp/README.md:25` — "`ResolvedMcp`, Agent materialization,
  and workspace MCP selection are later slices."
- `crates/plugin-manager/src/mcp.rs:17-18` — "It is not a `ResolvedMcp`."

No code combines `CompiledMcpConfiguration` + `StoredConfiguration` into a concrete
transport. The `ora-plugin-config` README (`plugin-config/README.md:28-29`) is explicit:
"The crate does not … pass configuration to Agent processes."

### 5.2 No MCP surface, no convergence, no writer

- **No MCP surface type.** `FilesystemSkillSurface` (`crates/effect/src/surface.rs:94-100`)
  is the only surface type; `MaterializationFormat` has only `skill_directory_v1()`
  (`surface.rs:62,64-66`); the registration validator rejects everything else
  (`agent_runtime/plugin_agent/effect.rs:67`).
- **No MCP Desired type.** `DesiredSkillState`/`SkillState` are skill-specific
  (`crates/effect/src/reconcile.rs` reads `SKILL.md`, computes skill digests). No
  `DesiredMcpState`.
- **No MCP convergence.** `converge_workspace_surfaces`
  (`effect_surface_registration.rs:22-58`) handles only `FilesystemSkillSurface`;
  `agent_effect_surface_declarations()` returns `Vec<FilesystemSkillSurface>`.
- **MCP config is plugin-global, not per-Workspace.** tavily's `apiKey` lives in
  `<data_dir>/plugins/data/official/ora-space.tavily-search/store.json`
  (`bootstrap.rs:2069-2070` test), not in the workspace.
- **MCP plugins never launch.** `permissions.rs:95` returns `Vec::new()` for
  `PluginContribution::Mcp`; `registration.rs:54-56` rejects MCP registration with "mcp
  plugins have no process and cannot register." The wire contract
  `InstalledPluginContribution::Mcp` (`crates/contracts/src/plugin.rs:34-35`) is a bare
  unit — no transport, no URL.
- **Surface crate is aware of MCP config but produces no surface.**
  `SurfaceDefinition::from_installed` (`crates/surface/src/definition.rs:83-110`) returns
  `None` for MCP plugins (`definition.rs:87`) yet still calls `compile_configuration_file`
  (`definition.rs:195-207`) — the natural seam for an MCP surface.

### 5.3 `McpArgument::WorkspaceContext` and `McpValueExpression::Setting` are unresolved

Both are compiled and stored but never consumed (`mcp/mod.rs:79-83`, `transport.rs:178-199`,
`transport.rs:241-262`). The intended resolution (per `mcp/mod.rs:81` doc): some consumer
loads `store.json`, substitutes `prefix + effective_value(id) + suffix` for each
`Setting`, resolves `WorkspaceContext` to the agent instance cwd, and produces a
`ResolvedMcp`. That consumer does not exist.

---

## 6. What the spec says

### 6.1 Active spec (implemented for skills; spec'd-but-unbuilt for MCP)

- **MCP is config-only; the agent plugin renders.** `specs/active/plugin/5-mcp.md#定位`
  (L5-9): "Ora 负责安装与解析 MCP 包；Agent 插件负责把 Ora 的规范化描述转换成 Claude
  Code、Codex、OpenCode 等目标 Agent 的配置格式."
- **`configure_agent` (spec'd active, NOT in code — grep: 0 matches).**
  `specs/active/plugin/5-mcp.md#agent-配置时序` (L261-291): before `start_agent`, Ora calls
  `ora/agent/configure_agent { agent_instance_id, cwd, revision, mcps: [ResolvedMcp] }`;
  the plugin idempotently reconciles Ora-managed entries, preserves user entries, plans
  then atomically replaces, returns applied revision + managed identity + fingerprint.
- **Use-time resolution.** `5-mcp.md#使用期解析` (L210-220, L238-245): "使用 MCP 前，Ora
  根据精确安装版本、当前插件 `store.json` 和 Agent Workspace 生成 `ResolvedMcp`";
  `context: workspace` resolves to the Agent instance's authoritative cwd.
- **Agent contract.** `4-agent.md#oraagentstart_agent` (L35-50, params `agent_instance_id`
  - `cwd`), `#oraagentstop_agent` (L52-62), `#oraagentlist_models` (L64-74); ACP
    adaptation is the plugin's job (`#acp-下的模型列表` L76-93). Same `cwd` for all sessions
    of an instance (`#agent-实例与-session` L25).
- **Workspace-scoped Effect state.** `specs/active/effect/1-category.md#定位` (L5-7);
  `WorkspaceEffectSpec { skills, mcps }` (`2-declaration.md#workspace-声明` L12-23);
  `AgentTarget = { workspace_id, agent_plugin_id }` (`#agenttarget` L69-89);
  `McpDefinition` is agent-agnostic, "Agent adapter 负责将它安全物化为目标格式"
  (`#mcpdefinition` L48-66). Watcher: durable reconcile requests, level-triggered, startup
  safety scan (`4-watcher.md#定位` L1-7, `#durable-reconcile-request` L49-82,
  `#desired-state-提交` L88-107, `#启动与安全扫描` L141-151). Reconcile timing:
  `2-declaration.md#reconcile-时序` L148-168.
- **`agent_effect_surface_declarations` is NOT in specs** — it appears only in `AGENTS.md`:
  "Register every consumer kind into the single declaration snapshot the convergence pass
  reads (`PluginApi::agent_effect_surface_declarations`)."

### 6.2 Planned (Effect v2 — `changes/`)

`specs/changes/effect/v2.md` introduces filesystem Surface adapters: `adapter_kind =
filesystem_directory | filesystem_document`, `format_kind = skill_directory.v1 |
mcp_config.v1` (`#05-effect_surfaces` L141-191); Surface↔Consumer with
`coordination_kind = uninterrupted | wait_for_idle_and_restart` (`#06` L193-229);
generation/phase status (`#09` L337-424, `#10` L426-524); ownership-recorded managed items
with `ManagedIdentity` markers (`#08` L278-332); cross-target Operations journal
(`#14` L887-1063). This is the spec's planned version of "Ora writes MCP config directly
to a filesystem target" — **planned, not active**.

### 6.3 Spec vs code (the divergence)

The active spec routes MCP through `configure_agent` (agent plugin renders + writes). The
**implemented** skills path routes through the Effect filesystem surface (Ora writes files
at a plugin-declared path, with plugin coordination via `effect/restart`). These are
different mechanisms. For MCP, **neither** is implemented:

| Mechanism                                              | Spec status       | Code status                            |
| ------------------------------------------------------ | ----------------- | -------------------------------------- |
| `configure_agent` (plugin renders → target config)     | active spec       | **absent** (0 refs)                    |
| Effect filesystem surface `mcp_config.v1` (Ora writes) | planned (v2)      | **absent** (only `skill_directory_v1`) |
| ACP `session/new` `mcp_servers`                        | — (schema exists) | **always empty**                       |

---

## 7. Marketplace & plugin truth

### 7.1 `ora-space.opencode` (agent) — `opencode-agent` repo

- **Manifest** (`opencode-agent:package.json` `ora.*`): `id=ora-space.opencode`,
  `kind=agent`, `main=./src/main.ts`, `engines {ora>=0.8.0, pluginApi=1, bun>=1.0.0}`,
  `contributes.agent.contractVersion=1`. (Note: `package.json.version=0.1.0` lags the
  marketplace `orax.toml.version=0.2.2` — the `orax.toml` version is install/verify
  authority.)
- **.orax v0.2.2 contents** (downloaded + unpacked, 13662 bytes): `orax.toml`, `main.js`
  (33054 bytes — `deno bundle` output, SDK vendored), `logo.svg`, `README.md`. No
  `assets/`, no `config.json`.
- **Launch** (`opencode-agent:src/services/opencode-client.ts`): spawns
  `<opencode-bin> acp --cwd <cwd>` (argv `[...extraArgs, "acp", "--cwd", cwd]`,
  `extraArgs` always `[]`). Binary resolved in `command.ts`: `ORA_OPENCODE_BIN` env, else
  `opencode.cmd` (Windows) / `opencode`. The plugin owns the CLI stdio: stdout NDJSON →
  re-parsed → forwarded as ACP frames; host frames → CLI stdin. Payload never parsed
  (`handlers/acp.ts`).
- **Effect surface (CRITICAL)** (`opencode-agent:src/handlers/effects.ts`):

  ```ts
  export const SKILLS_SURFACE: EffectSurfaceDeclaration = {
    workspaceRelativePath: ".opencode/skills",
    materializationFormat: "skill_directory.v1",
    coordination: "wait_for_idle_and_restart",
  };
  ```

  The opencode plugin declares **only** this surface. `SkillEffectCoordinator` tracks
  in-flight `session/prompt` turns from ACP frames (reads `method`+`id` only), idles,
  restarts the CLI (so it re-scans `.opencode/skills`), then replays. **No MCP-config
  surface; no code reads an MCP descriptor or writes `opencode.json`.**

### 7.2 `ora-space.tavily-search` (mcp) — `tavily-search-mcp` repo

- **Repo is descriptor-only**: `README.md`, `assets/config.json`, `logo.svg`, `orax.toml`.
  No `src/`, no `main.js`, no `package.json`, no release workflow.
- **`assets/config.json` verbatim** (`tavily-search-mcp:assets/config.json`):

  ```json
  {
    "schemaVersion": 1,
    "settings": {
      "apiKey": {
        "type": "string",
        "title": "API key",
        "description": "Tavily API key used to authenticate with the MCP server",
        "required": true
      }
    },
    "transport": {
      "type": "http",
      "url": "https://mcp.tavily.com/mcp",
      "headers": {
        "Authorization": { "setting": "apiKey", "prefix": "Bearer " }
      }
    }
  }
  ```

- **.orax v0.1.0 contents** (downloaded + unpacked, 1541 bytes): `assets/config.json`
  (395B), `logo.svg`, `orax.toml`, `README.md`. (Archive built on Windows — backslash
  entry separators; `unzip` warns but extracts correctly.)
- **What Ora must collect/resolve**: one stored setting `apiKey` (string, required) →
  `store.json`. Resolve to: HTTP endpoint `https://mcp.tavily.com/mcp` with header
  `Authorization: Bearer <stored-apiKey>` (literal `Bearer ` prefix + stored value).

### 7.3 opencode CLI's own MCP config (the target format)

Source: `https://opencode.ai/docs/mcp-servers` + `/docs/config`. opencode reads MCP from
the top-level `"mcp"` key of `opencode.json`/`.jsonc`. Config locations (merged, not
replaced; project config searches up from cwd to the nearest git dir):

| Scope       | Path                               |
| ----------- | ---------------------------------- |
| Global      | `~/.config/opencode/opencode.json` |
| Custom file | `$OPENCODE_CONFIG`                 |
| Project     | `<project-root>/opencode.json`     |
| Custom dir  | `$OPENCODE_CONFIG_DIR`             |
| Inline      | `$OPENCODE_CONFIG_CONTENT`         |

MCP entry schema — note opencode uses `"remote"`/`"local"`, **not** `"http"`/`"stdio"`:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "my-remote-mcp": {
      "type": "remote",
      "url": "https://my-mcp-server.com",
      "enabled": true,
      "headers": { "Authorization": "Bearer MY_API_KEY" },
    },
  },
}
```

### 7.4 The target opencode.json for tavily

The resolved tavily descriptor, rendered into the agent's project-root `opencode.json`,
should be:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "ora-space.tavily-search": {
      "type": "remote",
      "url": "https://mcp.tavily.com/mcp",
      "enabled": true,
      "headers": { "Authorization": "Bearer <stored-apiKey>" },
    },
  },
}
```

Because opencode merges config and the opencode plugin passes the **workspace** as
session `cwd` (§3), writing to `<workspace_root>/opencode.json` is the right target —
opencode searches up from cwd and finds it.

### 7.5 .orax layout summary

| kind                | .orax contents                                       | build                                   |
| ------------------- | ---------------------------------------------------- | --------------------------------------- |
| `agent`             | `orax.toml, main.js, logo.svg, README.md`            | `release.yml` `zip -j … dist/main.js …` |
| `mcp` (config-only) | `orax.toml, assets/config.json, logo.svg, README.md` | external/manual                         |

Marketplace conventions: `registry/<first-char>/<identifier>/` = `orax.toml + README.md +
logo.svg`; marketplace `orax.toml` adds `url` (release asset) + `sha256`; `resolver=1` is
the orax.toml schema version (distinct from `schemaVersion=1` for config.json and
`manifestVersion/contractVersion=1` in `package.json`); install target written
`namespace/identifier` (e.g. `official/ora-space.tavily-search`).

---

## 7.6 The bridge is the gap

The tavily README states the design intent explicitly: "an Agent plugin later turns the
installed descriptor into target-agent configuration." The opencode plugin v0.2.2 has not
implemented that step — its only Effect surface is `.opencode/skills`. Ora can install +
validate + store tavily, and spawn opencode, but nothing compiles the tavily descriptor
into `opencode.json`. **This is the seam to close.**

---

## 8. Three delivery channels (all currently unimplemented)

### Channel A — Effect filesystem surface (mirrors skills) — RECOMMENDED

Add an MCP consumer kind to the existing skills-style Effect surface machinery.

- **Ora side**: new surface type (e.g. `FilesystemMcpSurface`) + new
  `MaterializationFormat::mcp_config_v1()` in `crates/effect`; extend the validator
  (`agent_runtime/plugin_agent/effect.rs:67`) to accept it; teach `FilesystemSurfaceAdapter`
  (or a sibling `McpConfigSurfaceAdapter`) to render `ResolvedMcp` → the target config
  fragment (opencode `mcp` block) and write it (with `.ora-managed.json` ownership marker,
  atomic replace, quiesce/restart coordination).
- **Plugin side**: the opencode plugin declares a second surface, e.g.
  `{ workspaceRelativePath: "opencode.json", materializationFormat: "mcp_config.v1",
coordination: "wait_for_idle_and_restart" }`.
- **Pros**: exact "mirror skills" match; reuses proven `EffectWorker` +
  `FilesystemSurfaceAdapter` + convergence + ownership markers; satisfies the `AGENTS.md`
  "both directions, convergence in a worker" rule; the plugin only declares the path/format
  (no render logic).
- **Cons**: new surface type + format + reconciler is a real build; Ora owns the
  opencode-format rendering (acceptable — Ora already writes skill files at plugin-declared
  paths; the surface path keeps the plugin authoritative about _where_ and _how to
  coordinate_). This is the `changes/effect/v2.md` `mcp_config.v1` direction — building it
  makes the v2 proposal real for MCP.

### Channel B — ACP `session/new` `mcp_servers` (in-memory, thinnest)

Populate the existing ACP `NewSessionRequest.mcp_servers` field (schema:
`contracts/.../agent.rs:1011-1049`; `McpServer` enum `Stdio/Http/Sse/Acp` at
`agent.rs:2815`) with resolved MCP configs at `session/new` (`warm.rs:513-527`),
replacing `NewSessionRequest::new(cwd)`.

- **Pros**: smallest Ora change; no file writes; no plugin change if opencode's ACP mode
  honors `mcp_servers`; per-session scoping.
- **Cons**: does NOT match the user's "写入配置文件" requirement; **unverified that the
  opencode CLI's ACP mode actually launches MCPs passed via `session/new`** (the ACP schema
  defines the field, but opencode's behavior is an open question — see §10); diverges from
  the skills pattern and the spec's `configure_agent`.

### Channel C — spec's `configure_agent` (plugin renders → target config)

Implement the active spec: Ora calls `ora/agent/configure_agent { mcps: [ResolvedMcp] }`
before `start_agent`; the opencode plugin implements the handler to render `ResolvedMcp[]`
into `opencode.json` (scoped to `cwd`) with managed-identity markers + atomic replace.

- **Pros**: matches the active spec exactly; keeps target-format knowledge in the plugin
  (consistent with the opencode README's "nothing in Ora is hardcoded for this plugin"
  posture); reuses the plugin's existing restart coordination.
- **Cons**: Ora must add a new host→plugin method + the plugin must add a handler (bigger
  plugin change than Channel A's "declare a surface"); diverges from the implemented skills
  path (so the two consumer kinds — skills, MCP — would materialize through different
  mechanisms, complicating the convergence pass).

### Recommendation

**Channel A.** It is the literal "参考 skills 的配置方式" path, reuses the most existing
machinery, and aligns with the `AGENTS.md` convergence rule (a new Workspace-scoped consumer
kind registered into `agent_effect_surface_declarations`, converged in a worker for both
directions). The render logic (opencode `mcp` block) is small and the surface path stays
plugin-declared. Channel B is the fallback if opencode's ACP mode is confirmed to honor
`mcp_servers` and a file-less path is acceptable. Channel C is the spec-purist path if the
team wants one canonical mechanism and is willing to migrate skills to `configure_agent`
too.

**Regardless of channel, the universal prerequisite is `ResolvedMcp`.** Build that first.

---

## 9. Minimum closed loop — phased plan (Channel A)

**Phase 0 — `ResolvedMcp` (prerequisite, all channels).**
New type in `crates/plugin-config/src/mcp/` (or a new `resolve` module) that combines
`CompiledMcpConfiguration` + `StoredConfiguration` (via `ConfigurationService`) into a
concrete transport: resolve `McpValueExpression::Setting { id, prefix, suffix }` →
`prefix + effective_value(id) + suffix`; resolve `McpArgument::WorkspaceContext` → the
agent instance cwd; produce `ResolvedMcp { transport: ResolvedMcpTransport::Http{url, headers} | Stdio{command, args, env} }`.
Add a per-Workspace MCP selection (desired set) mirroring `WorkspaceEffectSpec.mcps`.

**Phase 1 — MCP surface + format (Ora side).**
In `crates/effect`: add `FilesystemMcpSurface` (or generalize the surface trait) +
`MaterializationFormat::mcp_config_v1()`; lift the `effect.rs:67` validator reject; add an
`McpConfigSurfaceAdapter` that renders `ResolvedMcp[]` → opencode `mcp` JSON fragment,
writes `<workspace_root>/opencode.json` with a `.ora-managed.json` marker, atomic replace.
Wire it into `converge_workspace_surfaces` + `reconcile_one` so MCP enters the same
durable-request + convergence loop as skills.

**Phase 2 — opencode plugin declares the MCP surface.**
In `opencode-agent:src/handlers/effects.ts`, add a second `EffectSurfaceDeclaration`:
`{ workspaceRelativePath: "opencode.json", materializationFormat: "mcp_config.v1",
coordination: "wait_for_idle_and_restart" }`. Extend `SkillEffectCoordinator` (or add a
sibling) to quiesce + restart on MCP-surface changes. Bump the plugin version + publish a
new `.orax` to the marketplace.

**Phase 3 — end-to-end.** Install opencode (new version) + tavily; save the tavily API key
in Ora settings; the Effect worker writes `opencode.json`; start a conversation; opencode
loads tavily and exposes web-search. Verify with a real tavily key.

> Scope guardrails: the AGENTS.md rule requires implementing **both directions** of the
> Workspace↔consumer pairing — new MCP consumer → all existing Workspaces, and new
> Workspace → all existing MCP consumers, both by convergence. Phase 1 must register the
> MCP surface into the single declaration snapshot the convergence pass reads, not into a
> second source.

---

## 10. Open questions & risks

1. **Does opencode's ACP mode honor `session/new` `mcp_servers`?** Decides whether Channel
   B is viable at all. Action: test against the opencode CLI, or read its ACP handler
   source. (External — not in the Ora repo.)
2. **opencode config merge vs. managed replace.** opencode merges config files by key. The
   spec's `configure_agent` requires preserving user entries and using a stable
   `managed_identity` to distinguish Ora-managed entries. opencode's `mcp` block is keyed
   by server name — using `ora-space.<name>` as the key gives a stable managed identity,
   but a user editing that exact key would collide. Define the ownership/merge policy for
   `mcp_config.v1`.
3. **`opencode.json` location.** Project-root `opencode.json` (in the workspace) is the
   natural target, but opencode also reads global `~/.config/opencode/opencode.json`. If
   the user wants MCP available across projects, global may be preferred — but that escapes
   the Workspace scope Ora materializes per-Workspace. Decide scope per product intent.
4. **Secret handling.** tavily's `apiKey` is Phase-1 `string` (stored in `store.json`); the
   spec reserves `secret` type for later (`2-settings.md#setting-声明` L75; reserved types
   rejected in MCP config, `mcp/mod.rs:108-113` + `mod.rs:190-203`). The resolved
   `Authorization: Bearer <apiKey>` lands in a workspace file — confirm this is acceptable
   for Phase 1 (it mirrors how opencode itself stores MCP keys in `opencode.json`).
5. **stdio MCP + `WorkspaceContext`.** tavily is HTTP (no cwd concern), but a stdio MCP
   (e.g. a future package) uses `McpArgument::WorkspaceContext` → the agent instance cwd.
   Confirm the cwd passed to opencode (session `cwd` = workspace) is the right "authoritative
   cwd" for stdio MCP launch by the opencode CLI.
6. **Surface crate seam.** `SurfaceDefinition::from_installed` (`surface/src/definition.rs:83-110`)
   already compiles MCP config but returns `None` for MCP. Decide whether MCP surface
   declaration is plugin-supplied (Channel A, mirrors skills) or host-derived (the surface
   crate knows MCP intrinsically). The skills precedent is plugin-supplied.

---

## 11. Source index

**Ora crates**

- Install: `crates/plugin-manager/src/install.rs:144,254,271,280,282-305,344-373,186-189`;
  `crates/plugin-manager/src/validation.rs:116-215,145-158,180-182`;
  `crates/plugin-manager/src/mcp.rs:20-22,29-69,73-128,35-39,56-60`;
  `crates/plugin-manager/src/lib.rs:42-59`
- Compile/store: `crates/plugin-config/src/mcp/mod.rs:6-7,37,42-50,55-58,62-68,72-75,79-83,87-97,108-113,130-149,153,161-184,189-203`;
  `crates/plugin-config/src/mcp/transport.rs:55,65,75,107,148,157,178-199,206,228,241-262,274,292,309,323`;
  `crates/plugin-config/src/mcp/README.md:25`; `crates/plugin-config/src/declaration.rs:11,13,21,126-127`;
  `crates/plugin-config/src/service.rs:196-215,251-272,300-371,390-449,452-499,517,696-749`;
  `crates/plugin-config/src/values.rs:9-15,17-25,28-82,146`; `crates/plugin-config/src/lib.rs:17-18`;
  `crates/plugin-config/README.md:4-6,28-29`
- Lifecycle/contracts: `crates/plugin-lifecycle/src/permissions.rs:95`;
  `crates/plugin-lifecycle/src/registration.rs:54-56`; `crates/plugin-lifecycle/src/state.rs:122-129,160`;
  `crates/contracts/src/plugin.rs:34-35,144`
- Agent runtime: `crates/backend/src/agent_runtime/plugin_agent/control.rs:58-63,155-161`;
  `crates/backend/src/agent_runtime/warm.rs:20,513-527`; `crates/backend/src/agent_runtime/mod.rs:312,853,874-878`;
  `crates/backend/src/agent_runtime/README.md:18`; `crates/backend/src/bootstrap.rs:42,183,2069-2070`;
  `crates/backend/src/agent_runtime/plugin_agent/README.md:32`;
  `crates/backend/src/agent_runtime/plugin_agent/effect.rs:5,54-89,67-69,119-134,240`
- Effect/skills: `crates/effect/src/filesystem.rs:120-121,268-301,314-327`;
  `crates/effect/src/surface.rs:62,64-66,94-100,115,129,175,179-180`; `crates/effect/src/reconcile.rs`;
  `crates/effect/src/tests.rs:354-382`; `crates/effect/src/lib.rs:38`;
  `crates/surface/src/definition.rs:83-110,87,195-207`;
  `crates/backend/src/effect_worker.rs:32,220-243,278-307,461-472,621-646,656-680,704,869-894,986-994`;
  `crates/backend/src/effect_surface_registration.rs:22-58,39-56,71,113-115`;
  `crates/backend/src/plugin.rs:243-251,465-507,503-505,514-522`;
  `crates/backend/src/skill_reconciliation.rs:30-119`;
  `crates/db/src/repository/effect/mod.rs:9,45-177,161-168,172,896`;
  `crates/db/src/repository/effect/mapping.rs:207-227`

**Specs** (`specs/`, nested repo, branch `main`)

- `specs/active/plugin/5-mcp.md` §定位 L5-9, §职责 L13-20, §包结构 L40, §配置格式 L52-59,
  §stdio L62-96, §http L133-169, §排他领域模型 L172-195, §工作目录 L129-130, §使用期解析
  L210-220 & L238-245, §agent-配置时序 L261-291, §agent-沙盒 L294-306
- `specs/active/plugin/4-agent.md` §oraagentstart_agent L35-50, §oraagentstop_agent L52-62,
  §oraagentlist_models L64-74, §acp-下的模型列表 L76-93, §agent-实例与-session L25,
  §实现边界 L98-104
- `specs/active/plugin/1-capability.md` §permissionsprocesssandbox L201-214, §解析与执行规则 L222
- `specs/active/plugin/2-settings.md` §setting-声明 L28-75, §运行上下文 L160-167
- `specs/active/effect/1-category.md#定位` L5-7; `2-declaration.md#定位` L5-8,
  §workspace-声明 L12-23, §mcpdefinition L48-66, §agenttarget L69-89, §reconcile-时序
  L148-168, §managed-state L176-203; `4-watcher.md#定位` L1-7, §durable-reconcile-request
  L49-82, §desired-state-提交 L88-107, §启动与安全扫描 L141-151
- `specs/changes/effect/v2.md` §05-effect_surfaces L141-191, §06-effect_surface_consumers
  L193-229, §08-effect_managed_items L278-332, §09-effect_surface_status L337-424,
  §10-effect_consumer_status L426-524, §14-effect_operations L887-1063
- `AGENTS.md` — `agent_effect_surface_declarations` + "both directions" convergence rule

**Marketplace & plugins**

- `ora-space/marketplace`: `registry/o/ora-space.opencode/{orax.toml,README.md}`,
  `registry/o/ora-space.tavily-search/{orax.toml,README.md}`
- `ora-space/opencode-agent`: `package.json`, `orax.toml`, `deno.json`,
  `src/main.ts`, `src/handlers/{lifecycle.ts,acp.ts,models.ts,effects.ts}`,
  `src/services/{opencode-client.ts,command.ts,ndjson.ts}`, `.github/workflows/release.yml`
- `ora-space/tavily-search-mcp`: `assets/config.json`, `orax.toml`, `README.md`
- opencode CLI docs: `https://opencode.ai/docs/mcp-servers`, `https://opencode.ai/docs/config`
- Verified .orax downloads: tavily v0.1.0 (1541B, sha256 matches), opencode v0.2.2 (13662B, sha256 matches)

**Verified in-session (grep)**: `configure_agent` → 0 matches in `crates/`;
`mcp_servers`/`mcpServers` → 0 Ora producers (only `NewSessionRequest::new(cwd)` at
`warm.rs:526`); `MaterializationFormat` → only `skill_directory_v1()` constructor; no
`mcp_config`/`McpConfig` materialization format.
