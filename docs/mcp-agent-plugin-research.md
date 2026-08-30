# MCP plugin to agent config: research & minimum-closed-loop design

> **Historical research snapshot.** The repository/source inventory remains useful, but its architecture conclusions—especially `configure_agent` before `start_agent`, Workspace selection, SecretRef, user-config merge, and the OpenCode adapter being out of scope—were superseded by the confirmed P1 boundary in [ADR-0015](./adr/0015-bound-the-first-mcp-agent-loop-to-an-ora-owned-file.md) and the current [MCP spec](../specs/active/plugin/5-mcp.md). Do not implement §§3, 8, or 9 as normative design.

> Ora can already sync the `https://github.com/ora-space/marketplace` registry, download + SHA-256-verify + extract + validate a `.orax` package, compile an MCP plugin's `assets/config.json` into a `CompiledMcpConfiguration`, and persist the user's Setting overrides (`apiKey`, etc.) in a plugin-global `store.json`. It can also install an `agent`-kind plugin (e.g. `ora-space.opencode`) and supervise its Deno process. What does **not** exist end-to-end is the _use-time_ half: resolving a compiled MCP transport against the user's stored Settings + the Agent instance's Workspace `cwd` into a strongly-typed `ResolvedMcp`, calling a `configure_agent` IPC that the agent plugin implements to render that `ResolvedMcp` into the target Agent's native config file in the working directory, and reconciling that materialization through the same Effect-worker + managed-identity + quiesce/restart coordination the Skills surface already uses. The spec (`specs/active/plugin/5-mcp.md`) defines this exact timing and responsibility split; the implemented `WorkspaceEffectSpec` carries only `skills` (no `mcps` map) and no `configure_agent` IPC method exists yet — so the minimum closed loop requires implementing the deferred `ResolvedMcp` + `configure_agent` slices the codebase explicitly calls "later slices" (`crates/plugin-config/src/mcp/README.md:32`).

## 1. Plugin management architecture (crate stack)

The plugin subsystem is layered across several `ora-*` crates with strict separation of concerns. None of the lower crates install, execute, or configure — they each own one slice.

### `ora-plugin-manifest` — parse & validate one `orax.toml`

Two parse-time forms distinguished by an internal `ManifestForm` enum (`crates/plugin-manifest/src/manifest.rs:17-21`):

- **Release form** (marketplace listing): parsed by `PluginManifest::parse` (`manifest.rs:50-54`). May carry download metadata (`url`/`sha256` top-level, or `[[targets]]`).
- **Installed form** (inside a `.orax`): parsed by `PluginManifest::parse_installed` (`manifest.rs:59-64`). Carries descriptive metadata only; `[[targets]]` is rejected; omitted `resolver` defaults to `SUPPORTED_RESOLVER = 1` (`manifest.rs:13`).

The manifest struct (`crates/plugin-manifest/src/manifest.rs:23-43`):

```rust
pub struct PluginManifest {
    pub(crate) resolver: u64,
    pub(crate) name: PluginName,
    pub(crate) title: String,
    pub(crate) namespace: PluginNamespace,
    pub(crate) kind: PluginKind,
    pub(crate) version: Version,
    pub(crate) description: String,
    pub(crate) homepage: Option<HomepageUrl>,
    pub(crate) license: Option<String>,
    pub(crate) url: Option<ReleaseUrl>,
    pub(crate) sha256: Option<Sha256Digest>,
    pub(crate) head: Option<PluginHead>,
    pub(crate) dependencies: Option<PluginDependencies>,
    pub(crate) workbench: Option<PluginWorkbench>,
    pub(crate) webview: Option<PluginWebview>,
    pub(crate) release_source: Option<PluginReleaseSource>,
    pub(crate) artifact: Option<PluginArtifact>,
}
```

All fields are `pub(crate)`; the public API is read-only accessors (`manifest.rs:202-299`). The crate does no serialization (`crates/plugin-manifest/README.md:44`).

The raw TOML schema (`manifest.rs:616-638`, `#[serde(deny_unknown_fields)]`) maps the **`identifier`** TOML key onto the `name` field — the TOML key for the plugin's name segment is `identifier`, not `name` (`manifest.rs:646-647`, `manifest.rs:723-726`). **Spec/code drift:** the MCP spec example (`specs/active/plugin/5-mcp.md:37-45`) writes `name = "github-mcp"`, but the parser expects `identifier`. The code is authoritative (unverified whether the spec is stale or illustrative-only).

Validation runs in `PluginManifest::from_raw_parts` (`manifest.rs:71-199`) in schema-declaration order so the first error is deterministic. Field-level validation includes: `resolver` must equal 1 (`manifest.rs:78-80`); `identifier` → `PluginName` (one or two dot-separated slug segments, max 128 bytes, validated via `ora_utils::Slug`, `crates/plugin-manifest/src/name.rs:5-38`); `namespace` → `PluginNamespace` (closed set, currently only `"official"`, `crates/plugin-manifest/src/enums.rs:6-38`); `version` → `semver::Version` (`tests.rs:564-577`); `url` → `ReleaseUrl` (HTTPS, query allowed, `crates/plugin-manifest/src/urls.rs:14-48`); `sha256` → `Sha256Digest` (exactly 64 hex chars, `crates/plugin-manifest/src/sha256.rs:5-40`).

### `ora-plugin-registry` — sync marketplace Git sources, build `registry_index.json`

`RegistrySource` (`crates/plugin-registry/src/source.rs:10-17`) models a marketplace Git source. `RegistrySource::try_from_git(url, branch, sources_root)` (`source.rs:70-82`) validates an HTTPS `RepositoryUrl` + `GitBranchName` and derives the checkout dir by stripping the scheme and joining the remainder: `https://github.com/ora-space/marketplace` checks out at `<sources_root>/github.com/ora-space/marketplace` (`source.rs:42-64`).

`RegistrySync::sync(git, source)` (`source.rs:119-156`) clones (absent checkout) or fetches+checks out+fast-forwards (existing), through an injected `gitlancer::Git` runner. Git commands assembled in `crates/gitlancer/src/git/sync.rs:52-112`.

`RegistryIndex::build_all(&[dirs], updated_at)` (`crates/plugin-registry/src/index.rs:43-72`) recursively collects `orax.toml` paths (sorted, no symlinks, `index.rs:215-238`), parses each via `PluginManifest::parse`, projects into `RegistryEntry::from_manifest` (`entry.rs:43-65`), sorts + dedups by `id` (`index.rs:63-64`). Bad files are skipped with a warning, never blocking the build (`index.rs:53-59`). The index is atomically persisted as `registry_index.json` (`index.rs:147-158`).

`RegistryEntry` (`crates/plugin-registry/src/entry.rs:10-39`):

```rust
pub struct RegistryEntry {
    id: PluginId,
    #[serde(default)] title: String,
    #[serde(default)] kind: String,
    namespace: String,
    version: Version,
    description: String,
    #[serde(default)] logo: Option<String>,
    #[serde(default)] release_targets: Option<Vec<String>>,
}
```

`id` is `PluginId` = `<namespace>/<name>` (`crates/domain/src/plugin_id.rs:19-64`), serialized as one opaque string (`plugin_id.rs:76-89`). Install-time re-read: `resolve_manifest_all` (`index.rs:134-144`) returns the full `PluginManifest` with release `url`/`sha256` by matching `namespace/name`.

### `ora-plugin-manager` — discover installed packages, checksum-verified install

`PluginManager::discover(data_dir)` (`crates/plugin-manager/src/lib.rs:48-77`) builds one immutable startup snapshot from `<data-dir>/plugins/installed/<namespace>/<name>/<version>`, highest version per id wins (`crates/plugin-manager/README.md:8-10`).

`Installer<D: HttpDownload>` (`crates/plugin-manager/src/install.rs:250-262`) owns the install path. `install_package` (`install.rs:296-373`): `download_package` (SHA-256 verified during download, `install.rs:548-578`) → cache archive under `<data_dir>/plugins/cache/<name>-<version>.orax` → reject `AlreadyInstalled` → `extract_archive(Zip, ...)` into a `tempfile::tempdir_in` staging dir with `package_extract_limits()` (`install.rs:329-338`) → `validation::validate(staging, manifest, None)` (`install.rs:339-340`) → targeted-release `[artifact]` target match (`install.rs:344-367`) → atomic `rename(staging → package_dir)` commit (`install.rs:368-372`).

`InstalledPlugin` (`crates/plugin-manager/src/validation.rs:76-90`) carries `contributes: PluginContribution` — a closed enum `Agent | Workbench | Webview | Skill | Mcp | Hook` (`validation.rs:27-58`). Kind and contribution are one value; `entrypoint()` returns `Some` only for Agent/Workbench, `None` for Mcp/Hook/Skill/Webview (`validation.rs:51-57`).

### `ora-plugin-config` — compile `assets/config.json`, persist Setting values

`ConfigurationService<FileSystem>` (`crates/plugin-config/src/service.rs:135-140`) compiles `assets/config.json` into one of three mutually-exclusive shapes via `compile_configuration_file` (`crates/plugin-config/src/mcp/mod.rs:142-173`): `CompiledConfigurationFile::Settings | Mcp | Hook`. The MCP shape yields `CompiledMcpConfiguration` (see §2). The only thing this crate writes to disk is per-plugin Setting _values_ in `store.json` at `<data_root>/plugins/data/<namespace>/<name>/store.json` (`service.rs:455-502`). **It explicitly does not persist MCP transport metadata or write to any agent working-directory config file** — `ResolvedMcp`, "Agent materialization," and "workspace selection" are deferred "later slices" (`crates/plugin-config/src/mcp/README.md:32`, `mod.rs:6-7`).

### `ora-plugin-lifecycle` — sole owner of plugin processes

`PluginLifecycle<RuntimeLauncher, StatusPublisher, NotificationSink>` (`crates/plugin-lifecycle/src/lib.rs:99-123`). State machine `ManagedPluginState<Runtime>` (`crates/plugin-lifecycle/src/state.rs:113-119`): `Stopped | Starting{attempt} | Running{attempt,runtime} | Failed{reason}`. MCP/Hook/Skill/Webview plugins have no process and remain `Stopped` forever (`crates/plugin-lifecycle/README.md:13-14`). `activate_plugin` (`lib.rs:223-286`) refuses kinds with `entrypoint().is_none()` via `NoProcess` (`lib.rs:237-240`); `validate_registration` (`crates/plugin-lifecycle/src/registration.rs:48-59`) hard-errors for Mcp. `permissions_for` (`crates/plugin-lifecycle/src/permissions.rs:89-98`) returns `Vec::new()` for Mcp — zero `--allow-*` flags.

### `ora-plugin-runtime` + `ora-process` — actual process spawn + tree-kill

`PluginRuntime::launch` (`crates/plugin-runtime/src/lib.rs:113-233`) spawns `deno run --no-prompt <permissions> <entrypoint>` with `cwd = package root`, wires 5 background tasks (reader/writer/stderr/supervisor/handshake). `crates/process/src/tree.rs` implements process-tree termination: Unix process-group `kill(-pgid, SIGKILL)`; Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (`tree.rs:104-117, 290-320`). The `PluginProcessHost` (`crates/plugin-lifecycle/src/childprocess.rs`) handles `ora/childprocess/*` requests so an **agent** plugin can ask the host to spawn subprocesses — available only to agent kinds (`permissions.rs:105-112`).

## 2. Plugin design & kinds (agent / mcp / ...)

### The `PluginKind` enum

`crates/plugin-manifest/src/enums.rs:52-60`:

```rust
pub enum PluginKind {
    Workbench,
    Agent,
    Webview,
    Skill,
    Mcp,
    Hook,
}
```

Manifest spellings (`enums.rs:78-88`): `workbench`, `agent`, `webview`, `skill`, `mcp`, `hook`. Unknown spellings rejected (`enums.rs:101-114`).

Kind dispatches behavior via `may_ship_targeted_artifact` (`enums.rs:62-75`): only `Hook` and `Agent` may declare `[[targets]]`/`[artifact]`. Kind also gates `[workbench]`/`[webview]` sections in `validate_kind_sections` (`manifest.rs:329-395`).

### Kind-by-kind meaning

- **`agent`** — drives a CLI agent. May bundle its CLI via targeted `[[targets]]` or resolve from PATH as a universal package (`manifest.rs:160-163`, `tests.rs:1270-1304`). Cannot carry `[workbench]`/`[webview]`. The OpenCode fixture uses `kind = "agent"`, `identifier = "ora-space.opencode"` (`tests.rs:265-290`).
- **`mcp`** — pure configuration plugin describing an MCP Server for an agent to use. No `main.js`, no Deno process, no Ora SDK call; the MCP Server is started/managed by the target Agent CLI (`specs/active/plugin/5-mcp.md:5-9`). Cannot carry `[workbench]`/`[webview]` (`tests.rs:179-202`). Cannot ship targeted artifacts (`enums.rs:70-75`). The actual transport/settings live in `assets/config.json` (`specs/active/plugin/5-mcp.md:47-59`), not declared in `orax.toml`.
- **`skill`** — accepted by the schema with no kind-specific section (`tests.rs:156-164`). Cannot carry `[workbench]`/`[webview]`.
- **`workbench`** — a panel/page plugin. May carry optional `[workbench]` section listing `main.js` method names (`crates/plugin-manifest/src/workbench.rs:11-27`).
- **`webview`** — embedded web surface. **Requires** `[webview]` with `start_url`, `allowed_origins` (`crates/plugin-manifest/src/webview.rs:18-40`).
- **`hook`** — "processless" contribution whose package **is** the native executable. Host never starts a Deno runtime (`enums.rs:47-51`). Installed hook **must** self-declare `[artifact]` (`manifest.rs:164-172`).

### The MCP data model (`crates/plugin-config/src/mcp/mod.rs`)

`CompiledMcpConfiguration` (`mod.rs:46-56`):

```rust
pub struct CompiledMcpConfiguration {
    pub schema_version: u32,
    pub settings: Option<CompiledDeclaration>,
    pub transport: McpTransport,
}
```

`McpTransport` — exclusive union; "illegal combinations are unrepresentable: stdio cannot carry a URL or headers, HTTP cannot carry a command, args, or env" (`mod.rs:58-64`):

```rust
pub enum McpTransport {
    Stdio(McpStdioTransport),
    Http(McpHttpTransport),
}
```

`McpStdioTransport` (`mod.rs:66-74`): `{ command: PortableRelativePath, args: Vec<McpArgument>, env: BTreeMap<String, McpValueExpression> }`. `McpHttpTransport` (`mod.rs:76-81`): `{ url: Url, headers: BTreeMap<String, McpValueExpression> }`.

`McpArgument` (`mod.rs:84-89`):

```rust
pub enum McpArgument {
    Value(McpValueExpression),
    WorkspaceContext,   // { "context": "workspace" }, resolved later to the Agent instance's authoritative cwd
}
```

`McpValueExpression` (`mod.rs:91-103`):

```rust
pub enum McpValueExpression {
    Literal(String),
    Setting { id: String, prefix: String, suffix: String },
}
```

Key transport rules (`crates/plugin-config/src/mcp/transport.rs`): `type` discriminator accepts only `"stdio"` and `"http"` — `sse` rejected (`transport.rs:62-68`). Stdio command must be a traversal-free relative path strictly below `assets/` (`transport.rs:153-175`); no PATH lookup (`npx`/`uvx`). HTTP: HTTPS-only, no userinfo/fragment/**query** (`transport.rs:113-138`). HTTP header values must be `{ "setting": <id>, "prefix"?, "suffix"? }` references — literal header values rejected (`transport.rs:220-238`). No `Sse` variant exists (`mod.rs:120-121`).

### MCP-kind install validation (`crates/plugin-manager/src/mcp.rs`)

`InstalledMcpDescriptor` (`mcp.rs:22-24`) holds `configuration: CompiledMcpConfiguration`. The module doc (`mcp.rs:1-3`) and struct doc (`mcp.rs:17-20`) state explicitly: **"It is not a `ResolvedMcp`: it says nothing about the user having filled Settings or any Agent having loaded the MCP."**

`validate_mcp` (`mcp.rs:31-79`): rejects shipped `main.js` (`mcp.rs:37-42`); requires `assets/config.json` to compile to `CompiledConfigurationFile::Mcp` (`mcp.rs:43-73`); for stdio, `validate_command_containment` (`mcp.rs:83-138`) resolves command to a regular file inside the package (no symlink escape; Unix execute bit checked).

The test fixture `discovers_mcp_package_with_http_transport` (`crates/plugin-manager/src/kind_tests.rs:221-260`) confirms the Tavily HTTP config (`https://mcp.tavily.com/mcp`, `Authorization: Bearer ${apiKey}`) compiles to `McpTransport::Http(...)` with `McpValueExpression::Setting{ id:"apiKey", prefix:"Bearer ", suffix:"" }`, and `configuration_declaration == Valid`.

## 3. Specs (specs/active/plugin/*)

All citations are to paths under `specs/` (a git submodule). `specs/AGENTS.md` governs only the `decisions/` tree (ADR naming/lifecycle). The observed layout is `active/`, `changes/`, `decisions/`, `drafts/` (empty). `changes/plugin/*` explicitly subordinate to `active/plugin/*` as authoritative (`specs/changes/plugin/1-capability.md:5-6`, `specs/changes/plugin/7-webview.md:5-6`). (unverified: the active→changes→drafts progression rule is inferred from directory names, not stated in any spec doc.)

### Plugin overview (`0-overview.md`)

A plugin is a Deno process running `main.js` (`0-overview.md:5`), starting with **zero Deno permissions** (`0-overview.md:7-15`). The model is user-space (plugin process) vs kernel-space (Ora host); the **Ora SDK is the syscall interface** — the only entry point for a plugin to request host capabilities (`0-overview.md:21`). Bundled binaries cannot be started by `main.js` directly; the host starts and lifecycle-manages them via SDK request (`0-overview.md:39-47`).

### Capability & storage layout (`1-capability.md`)

Distinct dirs under `~/.ora/plugins/`: `sources/`, `installed/<namespace>/<plugin_name>/<version>/`, `data/<namespace>/<plugin_name>/`, `logs/`, `cache/` (`1-capability.md:7-19`). Permissions are declared in `orax.toml` but **not** translated into `deno run --allow-*` — the Deno process stays zero-permission; declarations only control which Ora SDK methods the plugin may call (`1-capability.md:110-114`). The opencode agent manifest example (`1-capability.md:118-143`): `kind = "agent"`, `name = "ora-space.opencode"`, `[permissions.process] executables = ["bin/opencode"]`, `[permissions.process.sandbox] workspace = "read-write", network = true, environment = []`.

### Settings (`2-settings.md`)

`assets/config.json` is immutable code per install version; user-filled `store.json` values are runtime data, version-independent (`2-settings.md:5-7,22`). `store.json` at `~/.ora/plugins/data/<namespace>/<plugin_name>/store.json` (`2-settings.md:11-22`). Values referenced structurally via `{"setting":"<id>"}` (with optional `prefix`/`suffix`) (`2-settings.md:78-95`). Secret Settings may only appear in MCP stdio env vars or MCP HTTP headers (`2-settings.md:103`); forbidden in executables, command args, URLs, file paths, logs (`2-settings.md:104`). `{"context":"workspace"}` resolves to the current Agent instance's authoritative `cwd` (`2-settings.md:159-167`). Config module exposes `compile(package_root) -> CompiledPluginConfig` and `resolve(compiled_config, plugin_store, runtime_context) -> ResolvedPluginConfig | NeedsConfiguration` (`2-settings.md:149-157`).

### Registry (`3-registry.md`)

Plugin list pulled from marketplace repo into `~/.ora/plugins/sources/github.com/ora-space/marketplace`; entries under `registry/` (`3-registry.md:3-6`). Each plugin dir: `orax.toml`, `logo.svg`, `README.md`; a PR runs a GitHub Action validating the toml via the `orax` CLI (`3-registry.md:90-92`). Release assets: universal top-level `url`+`sha256` OR mutually-exclusive `[[targets]]` with Rust target triples (`3-registry.md:36-88`). A `registry_index.json` built atomically under `cache/` for UI display — no download URLs/digests in the index (`3-registry.md:131-154`).

### Agent plugin (`4-agent.md`)

"Agent 是 Ora 插件体系中最重要的插件类型" — the most important plugin kind (`4-agent.md:5`). Instance/Session model: `AgentPlugin → AgentInstance (agent_instance_id, cwd, sandbox) → Sessions`. Invariants: all Sessions under one instance share the same `cwd` (`4-agent.md:21-29`); `agent_instance_id` is Ora-allocated, not plugin-generated (`4-agent.md:27-29`).

Host→plugin methods the Agent plugin must implement (`4-agent.md:32-94`):

- `ora/agent/start_agent` — start/init an instance with `agent_instance_id` + `cwd` (authoritative working dir) (`4-agent.md:36-50`).
- `ora/agent/stop_agent` — stop/release an instance; must end all sessions/processes (`4-agent.md:52-62`).
- `ora/agent/list_models` — return models available in the instance's `cwd` context (`4-agent.md:64-74`).

The spec does not define an agent-specific `assets/config.json` schema (agent plugins run `main.js`). (unverified: whether agent plugins may optionally declare settings via `assets/config.json`.)

### MCP plugin (`5-mcp.md`) — load-bearing for the feature

A pure-configuration plugin. Runs no `main.js`, starts no Deno process, does not call the Ora SDK. The MCP Server is started/connected/managed by the target Agent CLI per its own config (`5-mcp.md:5-9`). MCP kind uses Ora's own config — not compatible with MCPB `manifest.json` or MCP Registry `server.json` (`5-mcp.md:9`).

**Responsibility split** (`5-mcp.md:12-20`): MCP package declares transport, Settings, bundled files. Ora validates, compiles, resolves. **The Agent plugin renders Ora's normalized `ResolvedMcp` into the target Agent's config format.** The Agent CLI creates the MCP Server process/connection. Critically: **the Agent plugin cannot directly read MCP's `assets/config.json`, `store.json`, or Ora DB** — the MCP config module hands it only the strongly-typed `ResolvedMcp` (`5-mcp.md:20`).

**Config format** (`5-mcp.md:52-60`): `assets/config.json` is strict JSON; root has `schemaVersion` (must be `1`), optional `settings`, and exactly one `transport` (stdio or HTTP). Unknown fields/versions/transports rejected at install.

**stdio transport** (`5-mcp.md:62-130`): `command` must be under `assets/` (no PATH lookup, no absolute paths, no symlink escape); `args` accept literals, `{"setting":"<id>"}` refs, and `{"context":"workspace"}` — Secrets forbidden in args (`5-mcp.md:112-120`); `env` values may be literals, ordinary Setting refs, or **Secret Setting refs** — the Agent adapter must convert these into the target Agent's safe env-reference form, **never writing plaintext into Workspace config** (`5-mcp.md:122-126`). Working dir fixed to the Agent instance's authoritative `cwd` (`5-mcp.md:128-130`).

**HTTP transport** (`5-mcp.md:132-169`): MCP Streamable HTTP only (not HTTP+SSE). `url` (HTTPS, no userinfo), `headers` (Secret refs only via Header Setting ref + optional `prefix`). No SSE, no multi-endpoint, no stdio fallback (`5-mcp.md:160-169`).

**Use-time resolution** (`5-mcp.md:212-245`): Ora generates from exact install version + current `store.json` + Agent Workspace:

```
ResolvedMcp { id, exact_version, transport: ResolvedStdio | ResolvedHttp, managed_identity }
```

e.g. `ResolvedMcp { id: "github-mcp", exact_version: "1.0.0", transport: ResolvedStdio { command: "/.../github-mcp/1.0.0/assets/server", ... }, managed_identity: "ora:mcp:github-mcp:1.0.0:..." }` (`5-mcp.md:224-235`). Resolution must: all required Settings resolvable; Secrets stay as `SecretRef`; `context: workspace` resolves to the current Agent instance's `cwd`; stdio command resolves to an absolute path in the exact install version; **never hand raw `config.json`, the whole `store.json`, or the plugin data dir to the Agent plugin** (`5-mcp.md:237-243`). Missing user config returns `NeedsConfiguration` (`5-mcp.md:245`).

**Agent configuration timing** (`5-mcp.md:262-291`) — THE LOAD-BEARING TIMING/ACTION:

```
读取 Workspace desired MCP set
        ↓
解析每个 MCP 为 ResolvedMcp
        ↓
ora/agent/configure_agent { agent_instance_id, cwd, revision, mcps: [ResolvedMcp] }
        ↓ 成功
ora/agent/start_agent { agent_instance_id, cwd }
```

`configure_agent` receives the **full expected set** and does idempotent reconcile — no lossy `add_mcp`/`remove_mcp` incremental events. The Agent plugin must (`5-mcp.md:282-289`): (1) add/update Ora-managed entries; (2) delete Ora-managed entries no longer in the set; (3) **preserve user-created config entries**; (4) use stable `managed_identity` to distinguish Ora vs user entries; (5) plan the full change then **atomically replace** the target config; (6) return the applied revision, managed identity, and config fingerprint. If any selected MCP cannot resolve or be safely materialized, `configure_agent` fails and Ora does **not** call `start_agent` (`5-mcp.md:291`).

**Workspace selection** (`5-mcp.md:248-258`): MCP selection is **Workspace-scoped**, not per-Session. All Sessions under one Agent instance share the same `cwd` and the same MCP config. Changing the desired MCP set produces a new revision; running Agent instances must be reconfigured + restarted — **v1 does not promise hot-plug** (`5-mcp.md:258`).

**States** (`5-mcp.md:308-320`): `Installed` → `NeedsConfiguration` (required Setting unavailable) → `Ready` (can produce `ResolvedMcp`) → `UnsupportedByAgent` / `ConfigurationFailed`.

### Effect spec (`specs/active/effect/2-declaration.md`)

The Desired State is the Workspace's desired Skill + MCP set: `WorkspaceEffectSpec { skills, mcps }` (`effect/2-declaration.md:9-28`). MCP enters as an agent-agnostic `McpDefinition { id, exact_version, transport: StdioDefinition|HttpDefinition, settings, secret_refs, definition_digest }` — the **Agent adapter** is responsible for materializing it (`effect/2-declaration.md:48-66`). The convergence target is an `AgentTarget = (workspace_id, agent_plugin_id)` (`effect/2-declaration.md:69-89`). Reconcile is blocked while any Session is Working/StoppingTurn, then Quiesce → session/close → atomically reconcile Skill+MCP definitions → session/resume (`effect/2-declaration.md:148-168`). `ManagedMcp { id, definition_digest, target_locator, applied_fingerprint }` alongside `ManagedSkill` (`effect/2-declaration.md:176-199`).

**Spec vs. implementation gap (critical):** the spec `WorkspaceEffectSpec` carries both `skills` and `mcps` maps; the **implemented** `WorkspaceEffectSpec` (`crates/effect/src/state.rs:89-91`) has **only `skills`** — the `mcps` map and `ManagedMcp` are not yet implemented.

## 4. Install / verify / marketplace-source flow

### End-to-end install flow

`PluginApi` (`crates/backend/src/plugin.rs:162-181`) owns a `MarketplaceSourceStore`, `ConfigurationService`, `Installer<ReqwestDownloader>`, the plugin `lifecycle`, and `registry_index_path = <data_dir>/plugins/cache/registry_index.json` (`plugin.rs:205`).

**1. Marketplace source add/persist.** `add_marketplace_source` (`plugin.rs:290-306`) delegates to `MarketplaceSourceStore::add` (`crates/backend/src/marketplace_sources.rs:60-77`), which validates URL+branch via `RegistrySource::try_from_git` and inserts a `PluginMarketplaceSourceRecord`. On first open, if the table is empty, the default `https://github.com/ora-space/marketplace`/`main` source is seeded (`marketplace_sources.rs:42-49`). Checkout root: `<data_dir>/plugins/sources` (`marketplace_sources.rs:37`).

**2. Git sync.** `sync_available_plugins` (`plugin.rs:338-375`): `prepared_registry_sources()` (`plugin.rs:446-469`) lists configured sources, attaches git proxy env where `use_proxy` is set; calls `RegistrySync::sync(&git, source)` (`plugin.rs:345-348`) for each. `RegistrySync::sync` (`crates/plugin-registry/src/source.rs:119-156`) clones (absent) or fetches+checks out+ff-pulls (existing). Git commands in `crates/gitlancer/src/git/sync.rs:52-112`.

**3. Build registry index.** `RegistryIndex::build_all(&registry_dirs, timestamp)` (`plugin.rs:351-354`); atomically written to `registry_index.json` (`plugin.rs:355-365`). `list_available_plugins` (`plugin.rs:257-274`) loads this cache.

**4. Install (download/verify/extract/register).** `install` / `install_with_progress` (`plugin.rs:621-679`) route to `install_package` (`plugin.rs:638-679`):

- `resolve_marketplace_release(plugin_id)` (`plugin.rs:725-754`) walks configured sources in precedence order, returns the first `RegistryIndex::resolve_manifest` hit + the winning source's `use_proxy` flag. Returns `PluginNotFound` when no checkout lists the id.
- `select_marketplace_release(manifest)` (`plugin.rs:760-767`) calls `select_release(manifest, HostTarget::from_option(current_host_target()))` — picks the universal URL/digest or the host-matched targeted artifact.
- A fresh `Installer::new(ReqwestDownloader::new(download_proxy))` is built per-install, honoring the winning source's proxy policy (`plugin.rs:653-654`).
- `installer.install[_with_progress](&manifest, release_source, &data_directory)` does the download→verify→extract→commit (see §1).
- `finalize_new_install(plugin_id)` (`plugin.rs:853-871`): `sync_plugin_skills` → `persist_discovered_plugin_skills` (`plugin.rs:908-963`) projects validated Skill metadata into the Skill catalog; `lifecycle.scan_plugins()` refreshes the installed-plugin snapshot so the new package is immediately usable without a restart (`plugin.rs:855-857`); `detect_hook_command_conflict` (`plugin.rs:878-905`) for Hook command-alias collisions.

### The VERIFY step, concretely

There is **no cryptographic signature** check; integrity is SHA-256 only. Layered across download, extraction, and validation:

**(a) Download checksum (SHA-256).** `download_package` builds a `DownloadRequest` with `Checksum::sha256(digest)` from the marketplace manifest's declared digest (`crates/plugin-manager/src/install.rs:555-576`). Mismatch → `InstallError::Download(DownloadError::ChecksumMismatch)` (`install.rs:799-831` test). Local-import path digest is self-declared — catches transit corruption only, not tamper (`install.rs:480-492`).

**(b) Safe extraction.** `extract_archive(ArchiveFormat::Zip, ...)` uses `ora-utils::archive` safe extractor (path-traversal/size limits), per AGENTS.md mandate (`install.rs:329-338`).

**(c) Manifest schema + kind policy + target match.** `validation::validate(package_root, manifest, logo)` (`crates/plugin-manager/src/validation.rs:123-251`): builds `PluginId` from namespace/name; reads/compiles `assets/config.json` via `ConfigurationService::configuration_file_from_package` (`validation.rs:139-149`); enforces kind/contribution exclusivity — only `mcp` packages may carry MCP transport, only `hook` packages may carry Hook config (`validation.rs:153-180`); dispatches per-kind validation; records `PluginConfigurationDeclarationValidity` (`validation.rs:211-237`).

**(d) MCP-kind-specific validation.** `validate_mcp` (`crates/plugin-manager/src/mcp.rs:31-79`): no `main.js`; config must compile to MCP shape; stdio command containment confirmed inside the package. No separate signature verification, no permissions-policy allowlist, no network reachability check — the compiled value is "static install-time truth only" (`crates/plugin-config/src/mcp/mod.rs:5-7`).

### DB schema

The only marketplace/install-related table is `plugin_marketplace_source`, created by migration `0006` (`crates/db/src/migration/schema/schema_v0006.rs:3-15`):

```sql
CREATE TABLE plugin_marketplace_source (
    url        TEXT PRIMARY KEY NOT NULL,
    branch     TEXT NOT NULL,
    use_proxy  INTEGER NOT NULL DEFAULT 0 CHECK (use_proxy IN (0, 1)),
    position   INTEGER NOT NULL CHECK (position >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

There is **no `plugin` or `installed_plugin` table**. Installed plugins are discovered from the filesystem by `PluginManager::discover` (`plugin.rs:248-253`). MCP plugin config metadata is **not** stored in any DB table — only the user's Setting overrides live in `store.json` on disk.

### IPC / contract surface

Rust contract DTOs (`crates/contracts/src/plugin.rs`): `AvailablePlugin { id, name, title, kind, namespace, version, description, logo, compatibility }` (`plugin.rs:209-230`); `InstallPluginRequest { plugin_id }` → `InstallPluginResponse { plugin_id, outcome: InstallOutcome }` (`plugin.rs:463-479`); `InstallOutcome` = `Installed | InstalledWithCommandConflict { conflict_plugin_id }` (`plugin.rs:487-501`). The MCP contribution on the wire is the fieldless `InstalledPluginContribution::Mcp` — serializes to `{"kind":"mcp"}`, carrying nothing but the kind discriminator (`plugin.rs:34-35`).

Frontend hooks (`packages/app-shell/src/state/hooks/`): `useAvailablePlugins` queries `client.plugin.listAvailable({})` (`use-available-plugins.ts:6-12`); `usePluginRegistrySync` mutates `client.plugin.syncAvailable({})` (`use-plugin-registry-sync.ts:6-14`); `useInstallPlugin(pluginId)` mutates `client.plugin.install({ pluginId })`, invalidates `installedPlugins` + `availablePlugins`, tracks progress via `usePluginOperationStore` (`use-install-plugin.ts:11-72`).

## 5. Marketplace & target plugins (opencode agent, tavily mcp)

### Marketplace repo structure

The `ora-space/marketplace` repo (default branch `main`, Apache-2.0) contains **only two top-level entries**: `LICENSE` and `registry/` (verified via the GitHub Contents API, `gh api repos/ora-space/marketplace/contents/`). There is **no top-level index file** (no `index.json`, no `registry.json`) and **no `.github/` CI** — the full recursive tree (`gh api repos/ora-space/marketplace/git/trees/main?recursive=1`) lists exactly `LICENSE`, `registry/`, and the per-plugin subdirectories below, with no `.github/` path. Discovery is purely by directory convention.

`registry/` is sharded by the **first character of the plugin identifier** into single-letter subdirs. Current shards:

- `registry/o/` — 5 plugins: `ora-space.claude`, `ora-space.codex`, `ora-space.opencode`, `ora-space.skillhub`, `ora-space.tavily-search`
- `registry/r/` — 1 plugin: `rtk-ai.rtk`

Every plugin directory has the same three files: `README.md`, `logo.svg`, `orax.toml`. The discovered Ora plugin manifest format in the marketplace is **`orax.toml`** (TOML).

**Two distinct manifests** (do not conflate):

1. **Registry/listing manifest** = marketplace `orax.toml` — packaging/listing metadata only (identifier, kind, version, download URL + sha256 of the `.orax` artifact, per-platform `[[targets]]`).
2. **Inner runtime manifest** = a file _inside_ the downloaded `.orax`. For agent plugins this is a `package.json` with an `ora` object. For MCP plugins it is `assets/config.json`.

### opencode AGENT plugin (`registry/o/ora-space.opencode/orax.toml`)

```toml
resolver = 1
title = "OpenCode"
identifier = "ora-space.opencode"
namespace = "official"
kind = "agent"
version = "0.3.0"
description = "Ora Space OpenCode Agent"
homepage = "https://github.com/ora-space/opencode-agent"
license = "Apache-2.0"

[[targets]]
target = "aarch64-apple-darwin"
url = "https://github.com/ora-space/opencode-agent/releases/download/v0.3.0/ora-space.opencode-v0.3.0-aarch64-apple-darwin.orax"
sha256 = "74dbe953e895e93a50cf887052150a6b57002a11e3722b39a9b56db68ab00dd5"

[[targets]]
target = "x86_64-unknown-linux-gnu"
url = "https://github.com/ora-space/opencode-agent/releases/download/v0.3.0/ora-space.opencode-v0.3.0-x86_64-unknown-linux-gnu.orax"
sha256 = "0a7ab3cb99ee24c64d714db4b43e604d2a2693f8b1cf21f99be13f5d0f963ff3"

[[targets]]
target = "x86_64-pc-windows-msvc"
url = "https://github.com/ora-space/opencode-agent/releases/download/v0.3.0/ora-space.opencode-v0.3.0-x86_64-pc-windows-msvc.orax"
sha256 = "2c9d4d264590ab77dfaaba5c25fd9e7357a99084271e4ee9f6269bbff53761a6"
```

- `kind = "agent"` — the agent plugin kind.
- `[[targets]]` — **agent plugins are native, per-platform**. Each entry pairs a Rust target triple with a release-artifact `url` (a `.orax` binary) and `sha256`. Three targets ship: aarch64-apple-darwin, x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc.
- Per its README (`github.com/ora-space/opencode-agent/contents/README.md`), the opencode plugin runs the OpenCode CLI through its native Agent Client Protocol mode (`opencode acp`) as a child process and streams models/sessions/responses to Ora via ACP. It registers `agent/start`, `agent/stop`, `agent/listModels` and emits `agent/acp` — verified against the plugin source `src/main.ts` (`github.com/ora-space/opencode-agent/contents/src/main.ts`), which overrides `onStart`, `onStop`, `onListModels`, and `onAcp` (no `configure_agent` override exists yet). The inner `package.json` (`github.com/ora-space/opencode-agent/contents/package.json`, verified via the GitHub Contents API) declares an `ora` object: `ora.contributes.agent = { "displayName": "OpenCode", "contractVersion": 1 }` alongside `ora.manifestVersion = 1`, `ora.id = "ora-space.opencode"`, `ora.kind = "agent"`, `ora.main = "./src/main.ts"`, and `ora.engines = { ora: ">= 0.8.0", pluginApi: 1, bun: ">= 1.0.0" }`.
- The source repo's `orax.toml` (`github.com/ora-space/opencode-agent/contents/orax.toml`, verified via the GitHub Contents API) carries only `resolver`/`title`/`identifier`/`namespace`/`kind`/`version`/`description`/`homepage`/`license` and does **not** contain the `url`/`sha256`/`[[targets]]` lines — a publish step injects them into the marketplace registry copy. (unverified: where this publish tooling lives.)

### tavily MCP plugin (`registry/o/ora-space.tavily-search/orax.toml`)

```toml
resolver = 1
title = "Tavily"
identifier = "ora-space.tavily-search"
namespace = "official"
kind = "mcp"
version = "0.1.0"
description = "Tavily web search MCP over Streamable HTTP"
homepage = "https://github.com/ora-space/tavily-search-mcp"
license = "Apache-2.0"

url = "https://github.com/ora-space/tavily-search-mcp/releases/download/v0.1.0/ora-space.tavily-search-v0.1.0.orax"
sha256 = "a8b58b0fc0a7c85fe774620682703149b4b6acbaa99303f399309558da282130"
```

- `kind = "mcp"` — pure configuration plugin.
- No `[[targets]]` — a single top-level `url` + `sha256` pointing to one non-platform-specific `.orax` artifact (configuration-only, no native binary).
- **The registry manifest carries only packaging metadata** — it does **not** contain the MCP server URL, transport, command/args/env, scopes, or auth. That metadata lives **inside the `.orax` package** at `assets/config.json`.

### MCP config metadata — `assets/config.json` (inside the tavily `.orax`; readable in the source repo at `github.com/ora-space/tavily-search-mcp/contents/assets/config.json`)

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

- `transport.type: "http"` — MCP **Streamable HTTP** transport (no `command`/`args`/`env` block — this is a _remote_ MCP).
- `url: "https://mcp.tavily.com/mcp"` — the remote MCP endpoint.
- `headers.Authorization` = `{ "setting": "apiKey", "prefix": "Bearer " }` — at runtime Ora resolves the header by looking up the `apiKey` setting value and prefixing `Bearer `. The key is never baked into the package or marketplace listing (per README).
- (unverified: the config.json format for a stdio MCP plugin could only be inferred — tavily is the only `kind=mcp` plugin in the marketplace and it is remote-http. The `transport.type` discriminator implies a union, but no stdio example exists to confirm `command`/`args`/`env`.)

### Consumption chain (from tavily README, decisive for the design)

> "A configuration-only MCP plugin for Ora that describes Tavily's remote MCP server. Ora installs and validates the package; **an Agent plugin later turns the installed descriptor into target-agent configuration**. This package does not ship `main.js` and does not start a Deno process."

1. Ora syncs marketplace, downloads the `.orax` from the `orax.toml` `url`, verifies `sha256`, unpacks.
2. Ora reads `assets/config.json` (transport + url + headers + settings schema).
3. The user supplies secret settings (apiKey) in Ora settings; Ora persists them in local `store.json`.
4. An **agent plugin** (e.g. opencode) reads the installed MCP descriptor + resolved settings and **turns that descriptor into the target agent's configuration**.

### Versioning and indexing

No central index file, no git tags, no CI. Versioning is field-based via `version` in each `orax.toml`. The `.orax` artifact URL embeds the version and lives as a GitHub Release asset on each plugin's _source_ repo. The marketplace repo only stores the `orax.toml` pointer + sha256. Indexing = directory walk of `registry/<first-char>/<org.plugin>/orax.toml`.

## 6. Agent config, working dir & the skills-config approach

### What an "agent" entity is

In Ora an **agent** is not a first-class persisted row; it is _an installed agent-kind plugin package that supplies one ACP CLI_. Every agent Ora can reach is supplied by a plugin package (docs/agent-runtime.md:7). The agent identity is the plugin's **package name** (the `name` segment of `<namespace>/<name>`), carried as an open string called `agent_ref` (e.g. `ora-space.opencode`) (docs/agent-runtime.md:7; `crates/backend/src/agent_runtime/connection.rs:54-68`). `AgentRef` is a validated newtype over a namespaced string, not a closed enum — because which agents exist is not knowable when Ora is built (docs/domain-models.md:42).

The supervised agent set is mutable and reconciled live: installing an agent plugin makes its agent reachable in the running process; uninstalling drops the supervisor (`crates/backend/src/agent_runtime/connection.rs:182-238`; docs/agent-runtime.md:8-9).

### The agent's WORKING DIRECTORY

- A **task's** authoritative working directory is the git worktree at `<worktree_root>/<workspace_id>` (docs/task-worktrees.md:22). Existing checkout paths are **never recomposed**; on session start/load the path is resolved live: task → Workspace id → stored Worktree branch name → `git worktree list --porcelain`, which is authoritative (docs/task-worktrees.md:48-50; docs/agent-runtime.md:11).
- A **project's main workspace** resolves its stored location against the bootstrap path base — the parent of `ORA_DATA_DIR` in Desktop (docs/agent-runtime.md:11).
- The working directory is **re-derived from the target on every request** and compared with the one the session was created against; a moved/recreated worktree retires the session (docs/agent-runtime.md:40).
- The supervisor starts the agent through the plugin: `PluginApi::attach_agent(plugin_id)` (`crates/backend/src/plugin.rs:513-526`) → `plugin_agent::attach` → `control::start_agent(runtime, cwd, host_version)` issues the IPC `agent/start` with params `{ cwd, hostVersion }` (`crates/backend/src/agent_runtime/plugin_agent/control.rs:155-174`; `connection.rs:717-727, 800-805`).

### Files in / around the working directory

- **Skill directories** (agent-declared surface, e.g. `.opencode/skills/<name>/`) — materialized by Ora (see below). Each contains a `SKILL.md` and resources, plus an Ora ownership marker `.ora-managed.json`.
- **`.ora-managed.json`** marker — the on-disk half of the dual ownership proof for an Ora-managed skill directory (`crates/effect/src/filesystem.rs:19-46`).
- **`.ora-effect-operations/`** — staging/backup journal directory under a surface root (`crates/effect/src/filesystem.rs:20, 64-84`).
- **Session history JSONL** — one append-only `.jsonl` per Session under the **configured sessions root** (NOT the working directory): `<sessions_root>/...` (docs/agent-runtime.md:49; `bootstrap.rs:178`).
- **Agent's own config file(s)** — Ora does **not** write a single agent config file today. The agent plugin owns its native config format in the cwd. The only Ora-written files in the cwd are the materialized skill directories. Plugin-global credentials/settings live in `store.json` under the Ora data dir, not the working dir.

### The SKILLS config approach IN DETAIL (the template MCP must mirror)

Skills are **not** written as a single config file listing enabled skills. They are **materialized as directories** onto an agent-declared filesystem surface, reconciled to a per-Workspace Desired State by an Effect worker.

**(a) Declaration — the agent plugin declares a surface.** At registration the agent plugin publishes `effect_surfaces` (a `PluginEffectSurface { workspace_relative_path, materialization_format, coordination }`). `registered_skill_surfaces` (`crates/backend/src/agent_runtime/plugin_agent/effect.rs:54-89`) converts each into a `FilesystemSkillSurface { workspace_relative_path: SurfacePath, materialization_format: MaterializationFormat::skill_directory_v1(), consumer: ConsumerId(plugin_id.canonical()), coordination }`. Example declared path: `.opencode/skills` or `.codex/skills` (`effect.rs:229`; `crates/backend/src/effect_worker.rs:869-873` test).

**(b) Persist + wake.** Immediately after `attach`, the connection calls `plugin_host.replace_agent_effect_surfaces(plugin_id, effect_surfaces)` (`connection.rs:814-818`). `PluginApi::replace_agent_effect_surfaces` (`plugin.rs:533-575`) keeps the latest declaration per canonical Plugin ID in a process-local map, then for **every** Workspace currently in the DB: resolves the workspace's local path, merges all consumer declarations into `SurfaceDescriptorSet` (`crates/effect/src/surface.rs:120-157`), and `effect_repository.replace_surfaces(...)` persists them. After the commit it wakes the Effect worker (`reconcile.notify()`) — wake-after-commit so a lost wake costs only a scan interval, never a reconcile (`plugin.rs:569-573`).

**(c) The Effect worker — the reconciler.** `EffectWorker` (`crates/backend/src/effect_worker.rs:113-126`) runs on a dedicated OS thread with its own current-thread Tokio runtime (`effect_worker.rs:159-193`). A pass does: `run_safety_scan` (re-arms blocked requests every 300s, `effect_worker.rs:246-270`); `converge_surface_registrations` (registers surfaces for Workspaces created after the last declaration, `effect_worker.rs:278-307`); `claim_due_reconcile_requests` (batch of 16, `effect_worker.rs:224-243`); for each claimed surface: `reconcile_one` (`effect_worker.rs:310-333, 461-534`).

**(d) The actual filesystem WRITE.** `reconcile_one` builds a `FilesystemSurfaceAdapter::new(workspace_id, workspace_root, surface_key, surface_path)` and a `Reconciler`, then `reconcile_surface` scans, plans, mutates, and finalizes (`effect_worker.rs:467-481`). The adapter (`crates/effect/src/filesystem.rs`):

- `ensure_surface_root()` — creates `<workspace_root>/<surface_path>` after proving every ancestor is an ordinary non-symlink directory inside the canonical workspace (`filesystem.rs:126-180`).
- `stage(snapshot, managed_identity, paths)` — `copy_directory` of the skill package into `.ora-effect-operations/<op>/staging`, writes `ManagedSkillMarker::current(...)` JSON to `staging/.ora-managed.json`, returns an `AppliedFingerprint` (`filesystem.rs:268-302`).
- `apply_create` / `apply_swap` / `apply_delete` — `fs::rename` the staging dir to `<surface_root>/<skill_name>` (create), or rotate previous→backup then staging→target (swap), or move target→backup (delete) (`filesystem.rs:314-376`).

**(e) Exact path + format.** Materialized path: `<workspace_cwd>/<surface_path>/<skill_name>/SKILL.md` + `<workspace_cwd>/<surface_path>/<skill_name>/.ora-managed.json` (proven by `effect_worker.rs:986-994` and `:1146-1156`: `workspace_root.join(".opencode").join("skills").join("grilling").join("SKILL.md")` and `.ora-managed.json`).

- `SKILL.md` = YAML front matter (`---\nname:\ndescription:\n---\n`) + Markdown body (`crates/skill-package/src/manifest.rs:52-172`; `crates/skill-package/src/scan.rs`, `SKILL_MANIFEST_FILE_NAME = "SKILL.md"`).
- `.ora-managed.json` = JSON `{ schema_version: 1, workspace_id, surface_key, managed_identity }` (`crates/effect/src/filesystem.rs:24-46`).

There is **no single skills config file** (no JSON/TOML "enabled skills list"). The agent discovers skills by scanning its declared surface directory. `.ora-managed.json` is the Ora-ownership marker separating Ora-managed dirs from user-written ("Preserved") ones (docs/effect-skill-state.md:23-30).

**(f) TIMING / ACTION that triggers the write.** The write is **not** triggered by session create or agent start directly. It is triggered by:

1. **A Desired generation change** — selecting/installing a skill commits a new Desired set + a reconcile request and wakes the worker (docs/effect-skill-state.md:38-45).
2. **Creating a Workspace/Project** — convergence registers surfaces for the new Workspace and wakes the worker so "the first prompt in a new task can run before its Skills exist" (`effect_worker.rs:997-1030`).
3. **Recovery on startup** — `recover()` reschedules work a previous process left unscheduled (`effect_worker.rs:200-217`).

Coordination with a **live** agent: before mutating a surface whose consumer is running, `PluginSurfaceCoordinator::quiesce` calls `effect/waitForIdle` per consumer; if any is busy the surface parks (`Blocked`, no timer retry — only a runtime event or the 300s safety scan re-arms it, `effect_worker.rs:613-646`). After a barriered write, `resume` calls `effect/restart` with the new generation and **detaches** the provider-side sessions that died with the replaced agent process (`effect_worker.rs:656-681`). Only a barriered reconcile detaches; a no-op resume does not cost the user their sessions (`effect_worker.rs:649-655`).

### The effect / skill state model

Three states (`crates/effect/src/state.rs`; docs/effect-skill-state.md:8-20):

- **Desired** — normalized complete set keyed by source kind, namespace, case-insensitive skill name. `WorkspaceEffect { workspace_id, generation, spec: WorkspaceEffectSpec }` where the implemented `WorkspaceEffectSpec { skills: BTreeMap<SkillSelectionKey, DesiredSkillState> }` (`state.rs:89-91, 113-119`).
- **Managed** — the DB ownership ledger. A random `ManagedIdentity` stays stable across content updates and ends only after safe Desired removal or surface retirement (docs/effect-skill-state.md:12-13). `ManagedSkill { managed_identity, workspace_id, surface_key, selection_key, ... }` (`state.rs:132-137`).
- **Observed / Preserved** — from each live filesystem scan, never persisted. An existing dir without matching ledger+marker stays Preserved even when bytes match a catalog source (docs/effect-skill-state.md:14-16).

### Distinct host-level configuration layers (none is the agent working-dir config)

- **`ora-user-config`** (`crates/user-config/src/lib.rs`) — generic SQLite key/value adapter with typed `ConfigKey` enum (`DeveloperMode`, `LogLevel`, `NetworkProxySettings`, `WorktreeRoot`). Pure host-level preferences.
- **`ora-runtime-settings`** (`crates/runtime-settings`) — only the process-wide log-level transaction coordinator. Narrow, log-level-only.
- **`ora-plugin-config`** (`crates/plugin-config`) — compiles `assets/config.json`; persists **plugin-global** stored setting values in `store.json` at `<data_root>/plugins/data/<namespace>/<name>/store.json` (NOT the agent working dir). The MCP `CompiledMcpConfiguration` is compiled here but **never persisted as MCP metadata**.

The boundary the spec draws (`specs/active/plugin/5-mcp.md:16-20`): the **MCP config module** validates/compiles/resolves; the **Agent plugin** renders `ResolvedMcp` into the target agent's config format; the Agent CLI actually launches/connects the MCP server. Ora never writes the agent-native MCP config file itself — the agent plugin does, via `configure_agent`.

### Where MCP config metadata becomes available — and the gap

MCP config metadata is compiled at install/validation time but currently goes nowhere toward an agent:

- The MCP configuration is compiled into `CompiledMcpConfiguration` (`crates/plugin-config/src/mcp/mod.rs:46-64`); `InstalledMcpDescriptor { configuration }` (`crates/plugin-manager/src/mcp.rs:21-24`) is held inside `PluginContribution::Mcp(...)` (`validation.rs:34`) on the in-memory `InstalledPlugin`.
- The **Settings subset** is surfaced to the user via `ConfigurationService`: `load_declaration` returns only the settings subset for an MCP package (`service.rs:377-390`); `save`/`get` persist user overrides to `<data_root>/plugins/data/<namespace>/<name>/store.json` (`service.rs:455-502`). That is the _only_ MCP-related host-owned persistence today.
- The **transport** (`McpTransport`) and resolved form (`ResolvedMcp`) are **deliberately not modeled anywhere**. The MCP module's own doc states: "Resolution against `store.json` (`ResolvedMcp`) is a later, separate step and is deliberately not modeled here" (`mod.rs:5-7`); the README lists "ResolvedMcp, ResolvedHook, Agent materialization, and workspace selection" as **later slices** (`crates/plugin-config/src/mcp/README.md:32`).
- The **agent-facing contribution on the wire** is the fieldless `{ kind: "mcp" }` (`crates/contracts/src/plugin.rs:34-35`); the frontend never learns transport, command, args, env, or URL.
- The frontend workflow contract already carries an MCP binding list: `WorkflowAgentConfig { ..., skills: WorkflowAgentSkillConfig[], mcps: WorkflowAgentMcpConfig[], prompt, ... }` with `schemaVersion: 3` (`packages/workflow-runtime/src/types.ts:63-81, 13-23`) and a read-only inspector rendering enabled MCPs (`packages/app-shell/src/features/workflow-run/run-act-agent-config.tsx:119-145`).

## 7. Ora conventions to follow

### Rust crate & code rules (AGENTS.md)

- **Crate naming**: crates under `crates/` are prefixed `ora-` (AGENTS.md:12).
- **Comments**: every non-trivial function gets a comment above the signature; inline-comment complex logic; explain "why" not "what"; English (AGENTS.md:3-4).
- **Design for Testability**: prefer DI, decoupled components, Traits for mocking, small pure functions (AGENTS.md:5).
- **Prefer Static Dispatch**: Generics + Trait Bounds over `Box<dyn Trait>` for monomorphization (AGENTS.md:6).
- **Make Illegal States Unrepresentable**: enums with associated data for state machines, not structs with optional fields (AGENTS.md:7). Relevant to MCP config states (Installed/NeedsConfiguration/Ready).
- **Backward Compatibility**: preserve compatibility for user-facing behavior, persisted data, public APIs, IPC/protocols; provide explicit migration/deprecation for breaking changes (AGENTS.md:8).
- **No opaque positional literals**: avoid `bool`/ambiguous `Option` params forcing callsites like `foo(false)`; prefer enums/newtypes/named methods. When unavoidable, use `/*param_name*/` argument-comment convention before `None`/bools/numeric literals (AGENTS.md:17-21).
- **Match exhaustiveness**: make `match` exhaustive, avoid wildcard arms (AGENTS.md:22).
- **Path handling**: never hardcode separators or string-concatenate paths; always use `Path`/`PathBuf`/`.join()` (AGENTS.md:23). Critical for writing MCP config into the agent's working-directory config file.
- **Trait doc comments**: newly added traits must include doc comments explaining their role (AGENTS.md:24).
- **Test assertions**: prefer comparing equality of entire objects over field-by-field; use `pretty_assertions::assert_eq` (AGENTS.md:25, 56-61).
- **Docs sync**: update the `docs/` folder when adding/changing behavior (AGENTS.md:26).
- **Module hygiene**: prefer private modules with explicitly exported public crate API; don't create one-off helper methods (AGENTS.md:27-28).
- **Module size limits**: target Rust modules under 500 LoC (excl. tests); files exceeding ~800 LoC get new functionality in a new module; when extracting, move related tests/docs toward the new implementation (AGENTS.md:29-35).
- **Time**: use local time, not UTC (AGENTS.md:36).
- **Logging**: use `ora-logging` wrapper macros (not `tracing` directly); use `ora_logging::clock::now_local` (AGENTS.md:37).
- **`ora-utils` for generic logic**: any logic independent of Ora domain concepts goes in `ora-utils` (`crates/utils`); must not depend on any other `ora-*` crate; for path validation/normalization/archive extraction, prefer `ora-utils::path` and `ora-utils::archive` (AGENTS.md:38-39).

### `crates/effect/AGENTS.md` — MCP-relevant convergence rule

`crates/effect/AGENTS.md:1` states a hard rule: when adding a Workspace-scoped consumer kind (Effect surfaces, MCP, anything materialized per Workspace), implement **both** directions of the pairing — new consumer → existing Workspaces, and new Workspace → existing consumers. Derive the second direction by **convergence in a worker**, never from a process start or any one-shot event. Register every consumer kind into the single declaration snapshot the convergence pass reads (`PluginApi::agent_effect_surface_declarations`), rather than adding a second source.

### Test rules (AGENTS.md)

- **Commands** (AGENTS.md:43-54): `task test` runs frontend + Rust workspace lint+test (long); prefer smallest relevant task while iterating; `task --list` is authoritative. `task format`, `task lint:frontend`, `task test:frontend`, `task lint:crates`, `task test:crates`, `task lint`, `task test`.
- **Rust tracing tests**: install a test-scoped subscriber/dispatcher with explicit `LevelFilter::TRACE` via `tracing::subscriber::with_default` / `tracing::dispatcher::with_default`, scoped to the current test thread, covering setup helpers/bootstrap/fixtures/smoke checks too (because `tracing` caches callsite interest) (AGENTS.md:56-61).
- **Frontend test gate**: tests run under `scripts/run-with-clean-stderr.mjs`; any React Testing Library stderr warning fails the whole run even when Vitest reports green. A test rendering anything calling `useTranslation` must import `appI18n` itself. Fully await every operation that can update React; wrap direct writes to external stores (e.g. Zustand) in `act`; handle promises/timers/animations at their real async boundary (AGENTS.md:65-71).

## 8. Minimum closed-loop design (THE KEY DELIVERABLE)

### (a) How to install the opencode agent plugin + tavily mcp plugin from the marketplace via Ora's existing install/verify flow

Both plugins install through the **same existing flow** with zero new install-path code. The default marketplace source `https://github.com/ora-space/marketplace` branch `main` is already seeded on first open (`crates/backend/src/marketplace_sources.rs:8-9, 42-49`).

**For opencode (agent, targeted release):**

1. `sync_available_plugins` (`crates/backend/src/plugin.rs:338-375`) → `RegistrySync::sync` (`crates/plugin-registry/src/source.rs:119-156`) clones the marketplace into `<data_dir>/plugins/sources/github.com/ora-space/marketplace`.
2. `RegistryIndex::build_all` (`plugin.rs:351-354`) scans `registry/o/ora-space.opencode/orax.toml`, parses it via `PluginManifest::parse`.
3. Frontend calls `client.plugin.install({ pluginId: "official/ora-space.opencode" })` (`packages/app-shell/src/state/hooks/use-install-plugin.ts:11-72`).
4. `install_package` (`plugin.rs:638-679`): `resolve_marketplace_release` (`plugin.rs:725-754`) finds the manifest; `select_marketplace_release` (`plugin.rs:760-767`) calls `select_release(manifest, HostTarget::from_option(current_host_target()))` (`crates/plugin-manager/src/install.rs:175-204`) — picks the host-matched `[[targets]]` entry (e.g. `x86_64-pc-windows-msvc` on Windows).
5. `Installer::install` (`install.rs:296-373`): downloads `https://github.com/ora-space/opencode-agent/releases/download/v0.3.0/ora-space.opencode-v0.3.0-x86_64-pc-windows-msvc.orax`, SHA-256-verified during download (`install.rs:548-578`); extracts into staging (`install.rs:329-338`); `validation::validate` (`install.rs:339-340`) — kind=`agent` validates `main.js` entrypoint containment (`validation.rs:254-299`); targeted-release `[artifact]` target match (`install.rs:344-367`); atomic `rename` to `<data_dir>/plugins/installed/official/ora-space.opencode/0.3.0/` (`install.rs:368-372`).
6. `finalize_new_install` (`plugin.rs:853-871`): `sync_plugin_skills` projects the package's validated Skill metadata into the Skill catalog (`plugin.rs:854`, `:908-963`); `lifecycle.scan_plugins()` refreshes the installed-plugin snapshot so the new package is immediately usable without a restart (`plugin.rs:855-857`); `detect_hook_command_conflict` checks Hook command-alias collisions (`plugin.rs:860-869`). It does **not** call `sync_plugin_agents`. The supervised-agent reconciliation happens one layer up: the IPC handlers `BootstrapBackend::install_plugin` / `install_plugin_with_progress` (`crates/backend/src/bootstrap.rs:427-445`) call `self.agent_runtime.sync_plugin_agents()` (`bootstrap.rs:432`, `:443`) **after** `plugin.install(...)` returns, which reconciles the supervised agent set (`crates/backend/src/agent_runtime/connection.rs:188-238`) so the freshly installed agent is immediately reachable in the running process. (`sync_plugin_agents` is also invoked at bootstrap, `bootstrap.rs:389`, and from `ConnectionSupervisors::start`, `connection.rs:178`; its docstring at `connection.rs:182-187` states installing a plugin must make its agent reachable without a restart.)

**For tavily (mcp, universal release):**

1. Same sync + index build. `registry/o/ora-space.tavily-search/orax.toml` parsed; it has a single top-level `url` + `sha256` (universal release, no `[[targets]]`).
2. Frontend calls `client.plugin.install({ pluginId: "official/ora-space.tavily-search" })`.
3. `select_release` picks the universal `ResolvedReleaseSource::Universal` (no host target needed, `install.rs:175-204`).
4. `Installer::install`: downloads the single `.orax`, SHA-256-verified, extracts, `validation::validate` — kind=`mcp` runs `validate_mcp` (`crates/plugin-manager/src/mcp.rs:31-79`): rejects `main.js`, requires `assets/config.json` to compile to `CompiledConfigurationFile::Mcp`, validates stdio command containment (N/A for HTTP transport); atomic `rename` to `<data_dir>/plugins/installed/official/ora-space.tavily-search/0.1.0/`.
5. `finalize_new_install`: `lifecycle.scan_plugins()` refreshes the snapshot; the MCP plugin is now discoverable as `InstalledPlugin` with `contributes: PluginContribution::Mcp(InstalledMcpDescriptor { configuration: CompiledMcpConfiguration { ... } })`.
6. The user fills the `apiKey` setting via `client.plugin.saveConfiguration` (`crates/backend/src/bootstrap.rs:312-316`) → `ConfigurationService::save` (`crates/plugin-config/src/service.rs:240-280`) → `write_store` atomic write to `<data_root>/plugins/data/official/ora-space.tavily-search/store.json` (`service.rs:455-502`), persisted as `{"schemaVersion":1,"revision":1,"values":{"apiKey":"tvly-..."}}` (confirmed by `service.rs:699-752` test).

**No new install-path code is required.** The existing install/verify flow already handles both `kind = "agent"` (targeted) and `kind = "mcp"` (universal) packages.

### (b) The exact timing/action that writes the tavily MCP config metadata into the agent's working-directory config file

The spec mandates a **two-step sequence** (`specs/active/plugin/5-mcp.md:264-280`):

```
(1) Create Agent instance ID + resolve Workspace desired MCP set
        ↓
(2) ora/agent/configure_agent { agent_instance_id, cwd, revision, mcps: [ResolvedMcp] }
        ↓ 成功
(3) ora/agent/start_agent { agent_instance_id, cwd }
```

**`configure_agent` is the exact timing/action.** It runs **after creating the Agent instance ID and BEFORE `start_agent`** (`5-mcp.md:264-280`). The agent plugin's adapter receives `ResolvedMcp[]` (the tavily MCP resolved to its transport + SecretRef + exact version) and must render it into the target agent's native config format and **atomically replace** the Ora-managed section of the config file in the Workspace `cwd`, preserving user-created entries and keying Ora entries by `managed_identity` (`5-mcp.md:282-289`).

**Reconciling the convergence rule (the `crates/effect/AGENTS.md:1` mandate vs the synchronous `5-mcp.md:264-280` timing).** These are two distinct trigger surfaces, not a contradiction:

- **Initial instance seed** — `5-mcp.md:264-280` describes the **first** materialization for a freshly created `AgentInstance`: `configure_agent` runs synchronously between instance-ID creation and `start_agent` to seed the MCP config before the agent process launches. This is per-instance initial seeding, not a process-start one-shot.
- **Workspace-wide convergence** — `crates/effect/AGENTS.md:1` and `specs/active/effect/2-declaration.md` govern the **pairing directions** the rule names: a _new MCP consumer → existing Workspaces_ and a _new Workspace → existing MCP consumers_. Those must be derived by convergence in the Effect worker (Quiesce → `session/close` → reconfigure → `session/resume`, `effect/2-declaration.md:148-168`), never from a process start, "because a consumer declaring at startup cannot see a Workspace created later, and the resulting gap is silent" (`crates/effect/AGENTS.md:1`).

The convergence rule's "never from a process start or any other one-shot event" targets the _Workspace-pairing_ direction, not the per-instance initial seed. The proposed 8(b) write site ("between `attach` and `start_agent`") is therefore the **initial-attach invocation of `configure_agent`** — the first materialization for a new instance — and does **not** replace the worker. The worker still owns every _subsequent_ Workspace-wide desired-MCP change for already-running instances (Quiesce → `configure_agent` → resume/restart), exactly as it already does for Skills. So the minimum closed loop must implement **both**: the initial-attach `configure_agent` call (§8(b) write site) and the MCP consumer kind registered into the single declaration snapshot the convergence pass reads (§8(e) gap 5). A design that ships only the synchronous attach call and omits the worker would reintroduce the silent gap the rule forbids (an MCP installed while Workspaces already exist would not reach those Workspaces until the next attach).

**Why MCP is not a literal "mirror" of skills.** The skills write is triggered by three **asynchronous worker** events — a Desired generation change, Workspace creation, startup recovery (`effect_worker.rs:200-217, 997-1030`; docs/effect-skill-state.md:38-45) — and Ora's own `FilesystemSurfaceAdapter` performs the filesystem write. The MCP trigger differs on **both** axes, not only the "who writes" axis:

- **WHO** — Skills: Ora's `FilesystemSurfaceAdapter` writes skill directories directly (`crates/effect/src/filesystem.rs:268-376`). MCP: Ora does **not** write the agent's MCP config file; the **Agent plugin** renders `ResolvedMcp` into the target agent's native config in the cwd (`specs/active/plugin/5-mcp.md:16-20, 211-235`). Ora's deliverable to the plugin is a strongly-typed `ResolvedMcp`; Ora never hands the plugin raw `assets/config.json`, the whole `store.json`, or the plugin data dir (`5-mcp.md:20, 243`).
- **WHEN (initial seed)** — Skills are **not** written synchronously at attach: the attach path only persists surfaces and wakes the worker (`crates/backend/src/agent_runtime/connection.rs:814-818`, `crates/backend/src/plugin.rs:533-575`), which performs the materialization on its thread. MCP, by contrast, the spec mandates a **synchronous** `configure_agent` between instance-ID creation and `start_agent` (`5-mcp.md:264-280`). This sync-at-attach asymmetry is spec-mandated and does not violate the convergence rule, because the rule governs the Workspace-pairing direction (handled by the worker) rather than the per-instance initial seed. For subsequent desired-MCP changes after the instance is running, MCP converges through the same async worker path as skills.

**`McpDefinition` → `ResolvedMcp` pipeline (the two are different stages, do not conflate).** The Desired-stage object living in the Workspace Effect state is `McpDefinition { id, exact_version, transport: StdioDefinition|HttpDefinition, settings, secret_refs, definition_digest }` (`specs/active/effect/2-declaration.md:47-66`) — the agent-agnostic normalized declaration. Resolution against the exact install version + the plugin's `store.json` + the Agent Workspace cwd turns it into `ResolvedMcp { id, exact_version, transport: ResolvedStdio|ResolvedHttp, managed_identity }` (`specs/active/plugin/5-mcp.md:213-235`), with Secrets kept as `SecretRef` and `context: workspace` resolved to the instance `cwd`. The object that crosses the `configure_agent` IPC boundary is the **resolved** `ResolvedMcp[]` — never the raw `McpDefinition`, never `CompiledMcpConfiguration`, never `assets/config.json` (`5-mcp.md:20, 237-243`). So the pipeline is: Desired `McpDefinition` (Effect spec, in `WorkspaceEffectSpec.mcps`) → resolve → `ResolvedMcp` (5-mcp.md) → `configure_agent` payload.

The **target MCP write site** that the design must add: a new step in `spawn_initialized_process` / `plugin_agent::attach` (`crates/backend/src/agent_runtime/plugin_agent/mod.rs:37-63`, `crates/backend/src/agent_runtime/connection.rs:717-784`) **between `attach` and `start_agent** — scoped to the Workspace cwd — that:

1. Reads the Workspace's desired MCP set (the `mcps` map to be added to `WorkspaceEffectSpec`, `crates/effect/src/state.rs:89-91`).
2. Resolves each selected MCP to `ResolvedMcp` from the exact installed version + the plugin's `store.json` + the Agent Workspace cwd (`5-mcp.md:212-245`).
3. Calls the new `ora/agent/configure_agent` IPC with `{ agent_instance_id, cwd, revision, mcps: [ResolvedMcp] }`.
4. Fails closed (does **not** call `start_agent`) if any selected MCP is unresolvable or unsupported by the agent adapter (`5-mcp.md:291`).

The closest existing write site to mirror for the atomic-write mechanics is `ConfigurationService::save` → `write_store` → `ConfigurationFileSystem::atomic_write` to `<data-root>/plugins/data/<ns>/<name>/store.json` (`crates/plugin-config/src/service.rs:455-502`, `crates/plugin-config/src/filesystem.rs:59-73`), using restrictive perms (`0o600` on Unix, `restrict_to_current_user` on Windows). The `McpArgument::WorkspaceContext` placeholder (`crates/plugin-config/src/mcp/mod.rs:84-89`) is the designated hook for injecting the agent cwd into stdio args.

### (c) Config file path + format

**Config file path:** the agent's working directory is the Workspace's location — for a local-filesystem Workspace, the resolved `PathBuf` git worktree at `<worktree_root>/<workspace_id>` (docs/task-worktrees.md:22). The MCP config file the agent plugin writes lives **inside this cwd** at a path owned by the agent plugin.

**Native config file for the opencode target (now grounded against the OpenCode CLI source, `github.com/sst/opencode`).** The OpenCode CLI reads project-local config from `opencode.json` (also `opencode.jsonc` / `config.json`) in the working directory, merged over a global `~/.config/opencode/opencode.json` (`packages/opencode/src/config/config.ts:272-274` global, `:422` project-local merge). MCP servers live under the **`mcp`** key (not the more common `mcpServers`) as a map keyed by server name. The value is a discriminated union (`packages/core/src/v1/config/mcp.ts`):

- **`local` (stdio)**: `{ type: "local", command: string[], cwd?: string, environment?: Record<string,string>, enabled?: boolean, timeout?: number }`. Note `command` is a **single array** of `[command, ...args]` (not a separate `command`+`args` split as in Ora's `McpStdioTransport`), env is `environment`, and `cwd` is optional (relative paths resolve from the workspace directory).
- **`remote` (HTTP)**: `{ type: "remote", url: string, headers?: Record<string,string>, enabled?: boolean, oauth?: ..., timeout?: number }`.

So the opencode adapter would render Ora's `ResolvedHttp` tavily transport into `{ type: "remote", url: "https://mcp.tavily.com/mcp", headers: { "Authorization": "Bearer <plaintext>" } }` under a stable `mcp[<managed_identity>]` key.

**Critical tension with the Secret rule (open design issue).** Ora's spec mandates Secrets stay as `SecretRef` and **never be written as plaintext into Workspace config** (`specs/active/plugin/5-mcp.md:122-126`); if the target agent cannot safely express a `SecretRef`, the adapter must return `SecretInputUnsupported` and `configure_agent` fails (`5-mcp.md:125-126`). OpenCode's native `mcp.local.environment` and `mcp.remote.headers` are **plaintext** `Record<string,string>` with no env-reference indirection in the public schema (`packages/core/src/v1/config/mcp.ts`). A faithful adapter therefore **cannot** render tavily's `apiKey` SecretRef into `opencode.json` without violating the no-plaintext rule, and would have to return `SecretInputUnsupported` unless opencode adds a secret-reference form. This is a real, unresolved blocker for the agent-side rendering of Secret-bearing MCPs.

**Scope of the "closed loop."** The opencode-agent Ora plugin (`github.com/ora-space/opencode-agent`) does **not yet implement `configure_agent`** — its `src/main.ts` overrides only `onStart` / `onStop` / `onListModels` / `onAcp` (verified via `gh api repos/ora-space/opencode-agent/contents/src/main.ts`), with no MCP-rendering handler. So Ora's minimum closed loop is closed only **up to the IPC boundary**: Ora resolves `ResolvedMcp` and calls `ora/agent/configure_agent`. The agent-plugin adapter's translation of `ResolvedMcp` into `opencode.json`'s `mcp` map — including the Secret-tension resolution above — is a separate, out-of-scope implementation task owned by the opencode-agent plugin, not by this design.

**`ResolvedMcp` format (per spec, `5-mcp.md:213-235`):**

```
ResolvedMcp {
    id: "ora-space.tavily-search",
    exact_version: "0.1.0",
    transport: ResolvedHttp {
        url: "https://mcp.tavily.com/mcp",
        headers: { "Authorization": SecretRef { setting: "apiKey", prefix: "Bearer " } }
    },
    managed_identity: "ora:mcp:ora-space.tavily-search:0.1.0:..."
}
```

Secrets stay as `SecretRef` (never stringified); the Agent adapter must convert them into the target Agent's safe env-reference form, returning `SecretInputUnsupported` if it cannot (`5-mcp.md:125-126`). The `.ora-managed.json` marker pattern from skills (`crates/effect/src/filesystem.rs:24-46`) is the ownership-proof template for distinguishing Ora-managed entries from user-created ones, keyed by `managed_identity`.

### (d) How the opencode agent plugin and the tavily mcp plugin relate in this loop

**Installing the agent plugin does NOT trigger MCP config attachment directly.** There is no manifest-level "this MCP belongs to that agent" binding. The relationship is realized per-Workspace, per-Agent-instance at `configure_agent` time (`specs/active/plugin/5-mcp.md:12-20, 248-258`):

1. **MCP is agent-agnostic at the package level.** An MCP package is a pure description of an MCP Server (transport + settings + bundled files). Any installed MCP plugin can be consumed by any Agent plugin, provided (a) the Workspace selects it, (b) it resolves to `Ready`, and (c) the Agent adapter can safely express its transport + SecretRefs (`5-mcp.md:20, 316-318`).
2. **Workspace selects MCPs.** MCP selection is a Workspace-level desired state (`5-mcp.md:248-258`). The Workspace's desired MCP set is the set of installed MCP plugins the user wants active in that working directory.
3. **Ora resolves each selected MCP to `ResolvedMcp`** using exact install version + plugin `store.json` + the Agent Workspace (`5-mcp.md:212-221`).
4. **Ora calls `configure_agent`** with `{agent_instance_id, cwd, revision, mcps: [ResolvedMcp]}` between instance-ID creation and `start_agent` (`5-mcp.md:264-280`).
5. **The Agent plugin (opencode) renders `ResolvedMcp`** into the target Agent's native config format and atomically writes it into the agent's working-directory config (the cwd), preserving user-created entries and keying Ora entries by `managed_identity` (`5-mcp.md:282-289`).
6. **The Agent CLI (opencode), started afterward via `start_agent`**, creates/connects the actual MCP Server per that config and manages the connection lifecycle (`5-mcp.md:12-18`).

So there is a **separate enable action**: the user selects desired MCPs at the Workspace level (producing a new Desired generation/revision). This is not triggered by installing the agent plugin; it is triggered by the user configuring the Workspace's desired MCP set. The `permissions.process.sandbox` of the _Agent_ plugin determines the sandbox the MCP stdio Server inherits (since the Agent CLI spawns it) — the MCP package itself declares no sandbox perms (`5-mcp.md:295-306`).

**End-to-end "configure an ALREADY-installed MCP for an agent" user-action sequence:**

1. **User opens the Workspace** and (in the workflow/agent config UI) toggles an installed MCP on. The existing frontend binding surface is the workflow Agent-node inspector: selecting an MCP appends `{ mcpId, enabled: true }` to `config.mcps` (`packages/app-shell/src/features/workflow-editor/workflow-inspector.tsx:441`), toggling maps to `:450`, removing to `:460`; the binding type is `WorkflowAgentMcpConfig { mcpId, enabled }` (`packages/workflow-runtime/src/types.ts:20-23`, `:72`). (Note: this is a per-workflow-node binding; the spec's Workspace-level desired-MCP set — the `mcps` map in `WorkspaceEffectSpec` — is **not yet implemented**, `crates/effect/src/state.rs:89-91`, so persisting that Workspace desired set is part of the missing work in §8(e) gap 2. The workflow binding is the closest existing mutation of an MCP desired set.)
2. **User fills the `apiKey` setting** via the settings UI → `client.plugin.saveConfiguration` (`crates/backend/src/bootstrap.rs:312-316`) → `ConfigurationService::save` (`crates/plugin-config/src/service.rs:240-280`) → `write_store` atomic write to `<data_root>/plugins/data/official/ora-space.tavily-search/store.json` (`service.rs:455-502`), persisted as `{"schemaVersion":1,"revision":1,"values":{"apiKey":"tvly-..."}}` (confirmed by `service.rs:699-752` test).
3. **User starts the agent** (new session / agent attach). `PluginApi::attach_agent(plugin_id)` (`crates/backend/src/plugin.rs:513-526`) → `plugin_agent::attach` → the new `configure_agent` step fires **between instance-ID creation and `start_agent`** (`specs/active/plugin/5-mcp.md:264-280`) with `{ agent_instance_id, cwd, revision, mcps: [ResolvedMcp] }`.
4. **Agent plugin renders** `ResolvedMcp` into its native cwd config (§8(c)), preserving user entries and keying Ora entries by `managed_identity` (`5-mcp.md:282-289`); on failure Ora does **not** call `start_agent` (`5-mcp.md:291`).
5. **Agent CLI starts** via `start_agent` and connects the MCP Server per the written config (`5-mcp.md:12-18`).
6. **Later desired-MCP change** (user toggles another MCP off) produces a new revision; the **Effect worker** — not the attach path — reconciles the already-running instance (Quiesce → `session/close` → `configure_agent` → `session/resume`), per §8(b) reconciliation and `effect/2-declaration.md:148-168`.

### (e) Gap analysis: what already exists end-to-end vs what is missing to close the loop

**Already exists (no new work needed):**

- Marketplace source seeding + sync (`crates/backend/src/marketplace_sources.rs`, `crates/plugin-registry/src/source.rs`).
- Install/verify/extract/commit for both targeted (agent) and universal (mcp) releases (`crates/plugin-manager/src/install.rs`).
- MCP-kind validation: no `main.js`, config shape, stdio command containment (`crates/plugin-manager/src/mcp.rs`).
- `CompiledMcpConfiguration` compilation from `assets/config.json` (`crates/plugin-config/src/mcp/mod.rs`).
- `InstalledMcpDescriptor` on the in-memory `InstalledPlugin` (`crates/plugin-manager/src/mcp.rs:21-24`).
- Settings persistence in `store.json` (`crates/plugin-config/src/service.rs:455-502`).
- Agent plugin supervision + ACP connection (`crates/backend/src/agent_runtime/connection.rs`).
- Skills Effect worker + `FilesystemSurfaceAdapter` + `ManagedSkill` ledger + `.ora-managed.json` marker (the skills half of the template, `crates/backend/src/effect_worker.rs`, `crates/effect/src/`).
- Frontend workflow contract carrying MCP binding list by `mcpId` + `enabled` (`packages/workflow-runtime/src/types.ts:63-81`).
- Agent contract verification (`verify_agent_contract`, `crates/backend/src/agent_runtime/plugin_agent/control.rs:110-152`).

**Missing to close the loop (the new work):**

1. **`ResolvedMcp` resolution** — no crate today turns `CompiledMcpConfiguration` + resolved `store.json` values + Workspace cwd into a concrete `ResolvedMcp`. The `McpValueExpression::Setting{id,prefix,suffix}` (`mod.rs:96-103`) and `McpArgument::WorkspaceContext` (`mod.rs:85-89`) remain unresolved expressions. The spec defines `ResolvedMcp` (`5-mcp.md:213-235`) and the settings spec defines `resolve(compiled_config, plugin_store, runtime_context) -> ResolvedPluginConfig | NeedsConfiguration` (`2-settings.md:149-157`), but the implementation is explicitly deferred (`crates/plugin-config/src/mcp/README.md:32`).
2. **`mcps` map in implemented `WorkspaceEffectSpec`** — the implemented `WorkspaceEffectSpec` (`crates/effect/src/state.rs:89-91`) has only `skills`; the spec defines both `skills` and `mcps` (`specs/active/effect/2-declaration.md:19-23`). `ManagedMcp` (`effect/2-declaration.md:176-199`) is not implemented.
3. **`ora/agent/configure_agent` IPC method** — the agent contract methods today are only `agent/start`, `agent/stop`, `agent/listModels`, `agent/acp`, `effect/waitForIdle`, `effect/restart` (`crates/backend/src/agent_runtime/plugin_agent/control.rs:13-19`, `effect.rs:17-18`). No `configure_agent` exists. The agent plugin's adapter rendering logic (turning `ResolvedMcp` into opencode's native config format) also does not exist.
4. **The wiring step** — inserting `configure_agent` into `spawn_initialized_process`/`plugin_agent::attach` (`connection.rs:717-784`) between `attach` and `start_agent`, scoped to the Workspace cwd, failing closed if any selected MCP is unresolvable. This is the _initial-attach_ invocation only (see §8(b) reconciliation); it does not substitute for gap 5.
5. **Convergence for MCP** — per `crates/effect/AGENTS.md:1`, the MCP consumer kind must implement both directions of the Workspace pairing (new MCP → existing Workspaces, new Workspace → existing MCPs), derived by convergence in a worker, registered into the single declaration snapshot. This is the _subsequent-changes_ path (§8(b)); the initial-attach call in gap 4 alone would leave the pairing directions unhandled and reintroduce the silent gap the convergence rule forbids.
6. **Agent-side rendering (out of scope, owned by the opencode-agent plugin)** — translating `ResolvedMcp` into `opencode.json`'s `mcp` map (§8(c)). The opencode-agent plugin does not yet implement `configure_agent` (verified, `src/main.ts`), and its native schema's plaintext `environment`/`headers` cannot express Ora's `SecretRef` — an open blocker (§8(c)). This design closes the loop only up to the IPC boundary; the native file write is a separate implementation task.

## 9. Gaps & open questions

1. **`ResolvedMcp` is not implemented.** The single biggest gap. The spec defines it (`specs/active/plugin/5-mcp.md:213-245`) and the settings spec defines the `resolve()` entrypoint (`2-settings.md:149-157`), but no crate implements it. The `McpValueExpression::Setting{id,prefix,suffix}` and `McpArgument::WorkspaceContext` remain unresolved expressions. Where should this live — extend `ora-plugin-config` (which explicitly defers it, `mcp/README.md:32`) or a new downstream crate? (unverified)
2. **`configure_agent` IPC does not exist.** The agent contract (`crates/backend/src/agent_runtime/plugin_agent/control.rs:13-19`) has no `configure_agent` method. The spec mandates it (`5-mcp.md:264-280`) but the implementation is absent. What is the IPC payload schema? How does it return the applied revision + managed identity + config fingerprint?
3. **opencode's native MCP config file format — resolved.** Grounded against the OpenCode CLI source (`github.com/sst/opencode`): the file is project-local `opencode.json` (or `.jsonc`/`config.json`) in the cwd (`packages/opencode/src/config/config.ts:272-274, 422`), MCP servers live under the `mcp` key as a discriminated `{ type: "local" | "remote" }` map (`packages/core/src/v1/config/mcp.ts`). See §8(c) for the full schema. **New open sub-issue:** the native `environment`/`headers` are plaintext `Record<string,string>`, which cannot express Ora's `SecretRef`, so a faithful opencode adapter would return `SecretInputUnsupported` for Secret-bearing MCPs (`5-mcp.md:125-126`) — this is an unresolved blocker for the agent-side rendering. Also: the opencode-agent Ora plugin does not yet implement `configure_agent` (no MCP handler in `src/main.ts`), so the agent-side rendering is out of scope for this design.
4. **`mcps` missing from implemented `WorkspaceEffectSpec`.** The spec defines `WorkspaceEffectSpec { skills, mcps }` (`specs/active/effect/2-declaration.md:19-23`); the implementation has only `skills` (`crates/effect/src/state.rs:89-91`). `ManagedMcp` (`effect/2-declaration.md:176-199`) is not implemented.
5. **Spec/code drift: `name` vs `identifier` in MCP spec example.** The MCP spec example (`specs/active/plugin/5-mcp.md:37-45`) writes `name = "github-mcp"`; the manifest parser (`crates/plugin-manifest/src/manifest.rs:646-647`) expects the TOML key `identifier`. The code is authoritative. (unverified whether the spec is stale or illustrative-only)
6. **Tavily stdio shape is (unverified).** tavily is the only `kind=mcp` plugin in the marketplace and it is remote-HTTP. The config.json format for a stdio MCP plugin (`command`/`args`/`env`) could only be inferred from the transport discriminator and the Rust model, not confirmed from a marketplace example.
7. **`home_directory` vs per-task worktree cwd.** The bootstrap `home_directory` passed to `agent/start` as `cwd` (`bootstrap.rs:180-192`, `connection.rs:554-566`) is the Ora home/data-dir path base, not the per-task worktree cwd. Whether per-session `session/new`/`session/load` separately carries the task worktree cwd to the agent (vs the process-start `home_directory`) was not fully traced. The spec's `configure_agent` takes `cwd` as an explicit param (`5-mcp.md:270`), so the design should treat the Workspace cwd as authoritative regardless. (unverified)
8. **Publish tooling for marketplace `orax.toml`.** The source repo's `orax.toml` does not contain `url`/`sha256`/`[[targets]]`; the marketplace registry copy has these appended. Where the publish step that injects release-artifact URL + sha256 lives is (unverified).
9. **`effect-skill-state.md` is possibly stale.** The doc claims "no product worker or real Agent runtime integration" (docs/effect-skill-state.md), but `EffectWorker` is implemented and wired in `bootstrap.rs:221-226`. Treat that specific doc claim as outdated.
10. **`docs/ora-concepts.md` and `docs/rtk-hook-plugin-research.md` do not exist** on branch `feat/mcp-agent-use` (confirmed by Glob). The user's auto-memory references them, but they were either removed by recent cleanup commits (`07e7fe11`, `a8679ca2`) or live on another branch. The authoritative domain reference is `docs/domain-models.md` + `docs/agent-runtime.md` + `docs/effect-skill-state.md`.
11. **Whether agent plugins may optionally declare settings via `assets/config.json`** is not explicitly stated. `4-agent.md` never references `assets/config.json` for the agent kind, while `2-settings.md` describes settings generically. (unverified)
12. **SDK API surface and packaging toolchain are not grounded.** `specs/changes/plugin/0a-sdk.md` is a one-line stub ("plugin code queries its own plugin version number"). The `orax` CLI authoring workflow is only mentioned for validation (`3-registry.md:90-92`). (unverified)

## 10. Sources

Consolidated, deduplicated list of every file/spec/URL cited above.

### Rust crates

- `crates/plugin-manifest/README.md`
- `crates/plugin-manifest/src/lib.rs`
- `crates/plugin-manifest/src/manifest.rs` (lines 13, 17-21, 23-43, 50-54, 59-64, 71-199, 202-299, 329-395, 402-411, 438-463, 474-568, 570-601, 616-638, 640-665, 646-647, 719-768, 779-812)
- `crates/plugin-manifest/src/enums.rs` (lines 6-38, 47-60, 62-75, 78-88, 101-114)
- `crates/plugin-manifest/src/target.rs` (lines 13-25, 27-36, 42-72)
- `crates/plugin-manifest/src/urls.rs` (lines 14-48, 50-122, 144-160)
- `crates/plugin-manifest/src/error.rs` (lines 12-31, 35-93, 95-137)
- `crates/plugin-manifest/src/name.rs` (lines 5-38)
- `crates/plugin-manifest/src/sha256.rs` (lines 5-40)
- `crates/plugin-manifest/src/webview.rs` (lines 18-40, 306-320, 397-436)
- `crates/plugin-manifest/src/workbench.rs` (lines 8-9, 11-27, 48-50)
- `crates/plugin-manifest/src/tests.rs` (lines 99-105, 156-164, 179-202, 219-242, 265-312, 316-354, 357-371, 564-577, 644-658, 1224-1238, 1270-1304, 1333-1355)
- `crates/plugin-registry/README.md`
- `crates/plugin-registry/src/lib.rs`
- `crates/plugin-registry/src/entry.rs` (lines 10-39, 43-65, 111-149, 156-161)
- `crates/plugin-registry/src/index.rs` (lines 13, 20-25, 33-35, 43-72, 81-89, 96-104, 134-144, 147-158, 215-238, 241-244, 313-328)
- `crates/plugin-registry/src/source.rs` (lines 10-17, 42-64, 70-82, 100-108, 119-156, 253-313)
- `crates/plugin-registry/src/logo.rs` (lines 11-29)
- `crates/plugin-registry/src/readme.rs`
- `crates/plugin-registry/src/error.rs` (lines 9-38)
- `crates/plugin-registry/src/host.rs` (lines 11-37)
- `crates/domain/src/plugin_id.rs` (lines 15-18, 19-64, 76-89, 122-134)
- `crates/plugin-manager/README.md` (lines 8-10, 51-54, 59-64)
- `crates/plugin-manager/src/lib.rs` (lines 21-40, 48-77, 214-220, 486-491)
- `crates/plugin-manager/src/install.rs` (lines 22-35, 49-56, 175-204, 238-244, 250-262, 264-273, 296-373, 384-428, 440-545, 480-492, 515-527, 548-578, 586-600, 608-629, 799-831)
- `crates/plugin-manager/src/validation.rs` (lines 27-58, 34, 51-57, 76-90, 101-113, 117-122, 123-251, 128-136, 139-149, 152-180, 181-210, 211-237, 254-299, 261-299)
- `crates/plugin-manager/src/mcp.rs` (lines 1-3, 17-20, 21-24, 22-24, 31-79, 37-42, 43-73, 83-138, 114-135)
- `crates/plugin-manager/src/hook.rs` (lines 17-19, 21-24, 32-33, 33-84, 89-154)
- `crates/plugin-manager/src/skill.rs` (lines 12, 15-19, 21-26, 29-170)
- `crates/plugin-manager/src/kind_tests.rs` (lines 179-202, 221-260, 264-301, 304-331, 334-367, 428-638)
- `crates/plugin-manager/src/tests.rs`
- `crates/plugin-config/README.md`
- `crates/plugin-config/Cargo.toml`
- `crates/plugin-config/src/lib.rs`
- `crates/plugin-config/src/service.rs` (lines 18-22, 36-46, 49-54, 56-73, 83-124, 135-140, 142-150, 156-160, 166-170, 172-179, 182-193, 196-215, 218-223, 226-237, 240-280, 283-297, 300-371, 377-390, 393-416, 455-487, 489-502, 505-517, 520, 564-611, 699-752, 744-752)
- `crates/plugin-config/src/declaration.rs` (lines 11, 13, 16-22, 25-35, 38-44, 46-53, 79-85, 91-134, 137-152, 242-247, 250-269, 284-372)
- `crates/plugin-config/src/values.rs` (lines 9-15, 28-82, 84-116, 129-144, 146)
- `crates/plugin-config/src/filesystem.rs` (lines 8-17, 20-86, 59-73, 75-85)
- `crates/plugin-config/src/mcp/README.md` (lines 27-31, 32)
- `crates/plugin-config/src/mcp/mod.rs` (lines 1-7, 5-7, 17-20, 29, 32-44, 39-44, 46-56, 50-54, 58-64, 61-81, 66-74, 76-81, 84-89, 87-88, 91-103, 96-103, 120-121, 127-139, 142-173, 175-182, 185-208, 196-201, 211-233)
- `crates/plugin-config/src/mcp/transport.rs` (lines 13-23, 25-33, 52-69, 72-101, 104-151, 113-118, 119-124, 125-130, 133-138, 153-175, 220-238, 241-262, 274-289, 292-304, 306-317)
- `crates/plugin-config/src/mcp/tests.rs` (lines 13-73, 14-18, 75-159, 161-177, 179-193, 196-219, 221-257, 234-256, 259-287, 289-315, 318-359, 362-400, 402-423, 427-457, 459-483, 486-500, 502-523, 525-542)
- `crates/plugin-config/src/hook/README.md` (lines 19-20)
- `crates/plugin-lifecycle/README.md` (lines 10-11, 13-14)
- `crates/plugin-lifecycle/src/lib.rs` (lines 99-123, 133-169, 208-211, 214-220, 223-286, 230-234, 237-240, 439-454, 486-491)
- `crates/plugin-lifecycle/src/registration.rs` (lines 20-61, 48-59)
- `crates/plugin-lifecycle/src/permissions.rs` (lines 53-63, 89-98, 95, 105-112)
- `crates/plugin-lifecycle/src/ports.rs` (lines 17-36, 28-35, 82-101, 114-122, 125-149, 134-140)
- `crates/plugin-lifecycle/src/state.rs` (lines 20-110, 53-95, 113-119, 122-129, 132-211, 160)
- `crates/plugin-lifecycle/src/launch.rs` (lines 31-138, 60-68, 69-80, 98-120, 130-137, 141-173, 176-237)
- `crates/plugin-lifecycle/src/runtime.rs` (lines 23-39, 60-70, 73-89, 91-168, 113-119, 124, 142-144, 148-157, 242-258)
- `crates/plugin-lifecycle/src/childprocess.rs` (lines 1-19, 34, 179-184, 229-247, 270-302)
- `crates/plugin-lifecycle/src/scan.rs`
- `crates/plugin-lifecycle/src/storage.rs`
- `crates/plugin-lifecycle/src/surface_closer.rs`
- `crates/plugin-lifecycle/src/uninstall.rs`
- `crates/plugin-lifecycle/src/connection.rs`
- `crates/plugin-lifecycle/src/data_dir.rs`
- `crates/plugin-lifecycle/src/data_plane_tests.rs`
- `crates/plugin-lifecycle/src/childprocess_tests.rs`
- `crates/plugin-lifecycle/src/storage_tests.rs`
- `crates/plugin-runtime/src/lib.rs` (lines 81-94, 113-233, 127-134, 135-137, 143-147, 195-226)
- `crates/plugin-runtime/src/state.rs` (lines 25-69, 106-113)
- `crates/plugin-runtime/src/tasks.rs` (lines 20-48, 51-80, 83-109, 112-170, 122-138, 139-165)
- `crates/plugin-runtime/src/protocol.rs` (lines 62-176, 146-160)
- `crates/plugin-runtime/src/host_requests.rs`
- `crates/plugin-runtime/src/codec.rs`
- `crates/process/README.md`
- `crates/process/src/traits.rs` (lines 51-68)
- `crates/process/src/tree.rs` (lines 36-41, 48-52, 69-83, 104-117, 119-136, 138-217, 219-225, 228-240, 290-320)
- `crates/effect/AGENTS.md` (line 1)
- `crates/effect/src/lib.rs`
- `crates/effect/src/filesystem.rs` (lines 19-46, 20, 24-46, 64-84, 126-180, 183-265, 268-302, 314-376)
- `crates/effect/src/surface.rs` (lines 120-157)
- `crates/effect/src/state.rs` (lines 89-91, 113-119, 132-137)
- `crates/effect/src/identity.rs`
- `crates/effect/src/planner.rs`
- `crates/effect/src/ports.rs`
- `crates/effect/src/reconcile.rs`
- `crates/effect/src/tests.rs`
- `crates/skill-package/src/lib.rs`
- `crates/skill-package/src/manifest.rs` (lines 52-172)
- `crates/skill-package/src/scan.rs`
- `crates/skill-package/src/source.rs`
- `crates/user-config/src/lib.rs` (lines 9-27, 30-59)
- `crates/runtime-settings/src/lib.rs`
- `crates/runtime-settings/src/traits.rs` (lines 10-50)
- `crates/runtime-settings/src/manager.rs` (lines 71-114)
- `crates/application/src/user_config.rs` (lines 63-153)
- `crates/backend/src/plugin.rs` (lines 162-181, 205, 248-253, 257-274, 290-306, 338-375, 383-424, 446-469, 513-526, 533-575, 569-573, 609-611, 621-635, 638-679, 687-719, 725-754, 760-767, 794-804, 808-846, 853-871, 878-905, 908-963)
- `crates/backend/src/marketplace_sources.rs` (lines 8-9, 37, 42-49, 60-77, 66-68, 69)
- `crates/backend/src/plugin_install_tests.rs`
- `crates/backend/src/plugin_configuration.rs`
- `crates/backend/src/bootstrap.rs` (lines 101, 160-171, 171-172, 178, 180-192, 221-226, 284-286, 289-318, 312-316, 322-329, 331-360, 363, 389, 418, 432, 443, 457, 470)
- `crates/backend/src/effect_worker.rs` (lines 31, 33-41, 43, 46, 79-110, 113-126, 159-193, 178-182, 200-217, 224-243, 246-270, 278-307, 310-333, 461-534, 467-481, 613-646, 649-655, 656-681, 869-873, 986-994, 997-1030, 1004-1030, 1146-1156, 1184-1259)
- `crates/backend/src/skill_reconciliation.rs`
- `crates/backend/src/agent_runtime/connection.rs` (lines 54-68, 54-74, 91-96, 116-128, 148-156, 165-180, 182-238, 188-238, 240-254, 410-433, 436-461, 554-657, 717-727, 717-784, 731-740, 800-805, 814-818)
- `crates/backend/src/agent_runtime/plugin_agent/mod.rs` (lines 37-63, 55-63)
- `crates/backend/src/agent_runtime/plugin_agent/effect.rs` (lines 17-18, 54-89, 67-72, 92-193, 228-244, 229)
- `crates/backend/src/agent_runtime/plugin_agent/control.rs` (lines 13-19, 19, 110-152, 155-174)
- `crates/backend/src/agent_runtime/plugin_agent/transport.rs`
- `crates/backend/src/agent_runtime/plugin_agent/inbound.rs`
- `crates/backend/src/agent_runtime/plugin_agent/tests.rs`
- `crates/contracts/src/plugin.rs` (lines 19-48, 34-35, 180-206, 209-230, 249-261, 264-276, 299-306, 309-320, 323-338, 341-347, 358-363, 463-479, 487-501, 504-517, 519-536, 607-663, 828-852)
- `crates/db/src/repository/marketplace_source.rs` (lines 7-17, 20-100)
- `crates/db/src/migration/schema/schema_v0006.rs` (lines 3-15)
- `crates/gitlancer/src/git/sync.rs` (lines 52-55, 58-66, 69-77, 80-93, 97-112)

### Specs (specs/ submodule)

- `specs/AGENTS.md` (lines 3-5, 7-9, 10, 11)
- `specs/active/plugin/0-overview.md` (lines 5, 7-15, 17-22, 21, 39-47, 47)
- `specs/active/plugin/1-capability.md` (lines 7-19, 22, 24-32, 110-114, 116-152, 118-143, 154-216, 213-214, 218-239, 232-237)
- `specs/active/plugin/2-settings.md` (lines 5-7, 11-22, 57-66, 75, 78-95, 103, 104, 136-145, 149-157, 159-167, 169-173)
- `specs/active/plugin/3-registry.md` (lines 3-6, 9-32, 36-88, 90-92, 96-100, 104-106, 121-127, 131-154)
- `specs/active/plugin/4-agent.md` (lines 5, 5-8, 10-30, 21-29, 27-29, 32-94, 36-50, 52-62, 64-74, 78-93, 96-113)
- `specs/active/plugin/5-mcp.md` (lines 5-9, 9, 12-20, 16-20, 20, 24-32, 36-45, 37-45, 47, 47-59, 49, 52-60, 62-130, 99-110, 112-120, 122-126, 125-126, 128-130, 132-169, 160-169, 172-194, 188-194, 198-208, 206-208, 211-235, 212-221, 212-245, 213-235, 224-235, 237-243, 245, 248-258, 258, 261-291, 264-280, 270, 282-289, 291, 295-306, 308-320, 316-318)
- `specs/active/plugin/6-workbench.md`
- `specs/active/plugin/7-webview.md` (lines 9-13, 55-57)
- `specs/active/project-workspace.md` (lines 14-19, 18, 22-30, 32-34, 36-38, 48-49, 359-373, 376-383)
- `specs/active/effect/1-category.md` (lines 5-7, 18-38)
- `specs/active/effect/2-declaration.md` (lines 9-28, 19-23, 47-66, 48-66, 62-66, 69-89, 148-168, 174-204, 176-199)
- `specs/active/effect/4-watcher.md`
- `specs/changes/plugin/0a-sdk.md` (line 1)
- `specs/changes/plugin/1-capability.md` (lines 5-6)
- `specs/changes/plugin/7-webview.md` (lines 5-6)
- `specs/decisions/desktop/core/effect/00000000-effect-system-foundation.md` (lines 14-40, 560-571, 786-815)

### Docs (docs/)

- `docs/agent-runtime.md` (lines 3, 7, 8-9, 11, 13-15, 40, 49, 128, 148-150)
- `docs/task-worktrees.md` (lines 3, 7-9, 9, 22, 48-50)
- `docs/task-workspace-files.md`
- `docs/effect-skill-state.md` (lines 1-5, 3-36, 8-20, 12-13, 14-16, 23-30, 32-36, 38-41, 38-45)
- `docs/settings.md` (lines 7-9)
- `docs/domain-models.md` (lines 3, 9-23, 25, 31, 35-40, 42, 54, 55, 56, 58, 60, 66)
- `docs/surface.md` (lines 3-9, 13-20, 22-24)
- `docs/application-contracts-boundary.md` (lines 3, 9, 11, 13-14, 15, 17, 18, 29-30, 61, 70-71, 72, 84)
- `docs/desktop-runtime.md` (lines 3, 7-9, 9-11, 19, 21, 66, 70-78, 81, 106)
- `docs/workflow.md` (lines 64-66)
- `docs/spec-management.md` (lines 3-6, 10-12, 16-18, 20-25, 29-32, 34-37, 40-53)
- `docs/frontend-contract-sdk.md`
- `docs/gitlancer-architecture.md`
- `docs/runtime-logging.md`
- `docs/database-migrations.md`
- `docs/database-repositories.md`
- `docs/BRAINSTORM.md`
- `docs/agents/issue-tracker.md`
- `docs/agents/triage-labels.md`
- `docs/agents/domain.md`

### TypeScript packages

- `packages/contracts/src/plugin.ts` (lines 111-114, 168, 184-186)
- `packages/contracts/src/endpoints.ts` (lines 643-646, 651-654, 678, 755-758, 763-766)
- `packages/workflow-runtime/src/types.ts` (lines 13-23, 63-81)
- `packages/workflow-runtime/src/index.ts`
- `packages/app-shell/src/features/workflow-run/run-act-agent-config.tsx` (lines 119-145)
- `packages/app-shell/src/state/hooks/use-install-plugin.ts` (lines 11-72)
- `packages/app-shell/src/state/hooks/use-marketplace-sources.ts` (lines 6-63)
- `packages/app-shell/src/state/hooks/use-available-plugins.ts` (lines 6-12)
- `packages/app-shell/src/state/hooks/use-plugin-registry-sync.ts` (lines 6-14)
- `packages/app-shell/src/state/hooks/use-update-plugin.ts` (lines 11-53)

### Marketplace (GitHub URLs; all structural claims verified via the GitHub Contents/Trees API, `gh api repos/.../contents/` and `.../git/trees/main?recursive=1`)

- `https://github.com/ora-space/marketplace` (default marketplace source, `main` branch; top-level: `LICENSE`, `registry/` only — no index file, no `.github/`, confirmed via `gh api repos/ora-space/marketplace/contents/` and `gh api repos/ora-space/marketplace/git/trees/main?recursive=1`)
- `registry/o/ora-space.opencode/orax.toml` — opencode agent plugin manifest
- `registry/o/ora-space.opencode/README.md`
- `registry/o/ora-space.tavily-search/orax.toml` — tavily MCP plugin manifest
- `registry/o/ora-space.tavily-search/README.md`
- `registry/o/ora-space.claude/orax.toml`
- `registry/o/ora-space.codex/orax.toml`
- `registry/o/ora-space.skillhub/orax.toml`
- `registry/r/rtk-ai.rtk/orax.toml`

### opencode-agent Ora plugin source (`github.com/ora-space/opencode-agent`; verified via GitHub Contents API)

- `orax.toml` (source-repo copy: metadata only, no `url`/`sha256`/`[[targets]]`)
- `package.json` (`ora` object: `manifestVersion:1`, `id`, `displayName:"OpenCode"`, `kind:"agent"`, `main`, `engines`, `contributes.agent = { displayName, contractVersion:1 }`)
- `README.md` (`opencode acp` ACP bridge behavior)
- `src/main.ts` (overrides `onStart`/`onStop`/`onListModels`/`onAcp`; **no `configure_agent` override**)

### OpenCode CLI source (`github.com/sst/opencode`; verified via GitHub Contents API — the _target_ agent's native config)

- `packages/opencode/src/config/config.ts` (lines 272-274 global config paths, 422 project-local merge, 246-268 `$schema`)
- `packages/core/src/v1/config/mcp.ts` (the `mcp` map value schema: `Local` stdio `{ type:"local", command:string[], cwd?, environment?, enabled?, timeout? }` / `Remote` http `{ type:"remote", url, headers?, oauth?, enabled?, timeout? }`)

### Tavily MCP source

- `https://github.com/ora-space/tavily-search-mcp` (source repo; `orax.toml` without release metadata; `assets/config.json` with transport/settings)
- `https://github.com/ora-space/tavily-search-mcp/contents/assets/config.json` — the MCP config descriptor
