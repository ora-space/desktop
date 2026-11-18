# Session MCP

English | [中文](session-mcp.zh.md)

Ora delivers configured MCP plugins as **Session runtime input**: the ACP `mcpServers` list of each
`session/new` and `session/load`. MCP is never an Effect Resource and never a Workspace file. Every
frame that sets up a provider session (`startSession`, the attach a prompt performs, a rebuild,
an agent switch, a workflow start and a live refresh) uses one Session setup snapshot. Pure resolution lives in
`crates/agent-runtime/src/mcp_resolver`; Desktop sources and health remain in Backend.

## Selection

Each Session persists a `SessionMcpSelection`:

- **Automatic** (ordinary chats): every installed, statically valid MCP plugin whose configuration is
  complete. Incomplete plugins are left out without failing the rest.
- **Explicit** (workflow Agent nodes): only the node's `mcps` bindings with `enabled: true`. An empty
  list delivers no servers. A selected plugin that is missing, invalid or incompletely configured
  fails setup; unselected plugins are filtered out before any check. Graphs without `mcps` mean an
  empty selection.

Installing an MCP plugin only adds it to the global catalog; a workflow node's selection is the
authorization decision. The selection is stored on the Session row when it is created and is reused
through restore, rebuild, agent switching and refresh; recovery never reconstructs it from workflow
metadata, so missing metadata cannot widen an explicit selection. Sessions that existed before
migration `0011` have an empty explicit selection. Editing a workflow draft never changes an
existing run.

## Building the snapshot

Server names are canonical plugin ids (`<namespace>/<identifier>`), sorted. A snapshot is
all-or-nothing: the runtime never sends a partial list.

- Stdio maps to ACP `McpServer::Stdio`; the command is re-checked as an ordinary file inside the
  current package version.
- HTTP maps to `McpServer::Http` and requires the agent to advertise HTTP MCP capability
  (`mcp_http_capability_missing` otherwise).
- `{ "context": "workspace" }` becomes the Session's absolute cwd; a literal `"."` stays `"."`.
  Environment variables and headers become ACP name/value lists.
- A non-empty set requires `session/load` support, because that is the only frame that can carry a
  changed set to a running Session. If the agent cannot load sessions, setup fails after ACP
  `initialize` and before any session setup frame is sent, including the `session/new` of a rebuild:
  a rebuild carries the real snapshot or does not happen.

## Live refresh

Live Sessions keep their Desired and Active MCP revisions in memory. Plugin install, update and
uninstall, and saving, clearing or recovering plugin settings, send a secret-free wakeup. An idle
Session runs `session/load` immediately; a busy one refreshes after the current prompt, and new
prompts wait for the refresh. Success advances the Active revision, and a newer Desired that arrives
meanwhile stays pending. Failure blocks only that Session, and the next prompt retries rather than
using old configuration. Stopped Sessions do not refresh in the background. Changes to plugins
outside an explicit selection do not change that Session's revision.

The shared revision also carries an explicit host generation. Existing hosts construct generation
zero; a host using ordered updates supplies the generation before resolving/sending. Even when the
member identities are unchanged, a newer generation requires its own successful convergence. An old
response cannot make that newer target current. The generation contains no Settings digest.

MCP refresh, Skill Effect mutation and agent replacement share one Agent Session Barrier so new
prompts wait for a safe point. They share no Effect state.

## Secrets and files

Setting values may exist in the configuration store, a short-lived in-memory snapshot and the ACP
frame sent to the agent. They never enter Effect state, SQLite, Workspace files, logs, errors, UI
DTOs, revision digests or agent environment variables. Errors name the plugin id, setting id,
transport and a stable code only.

Ora never creates, modifies or deletes `.mcp.json`, OpenCode JSON/JSONC, sidecars, Git exclude files
or any other Workspace path for MCP; existing user-authored MCP configuration is left alone.

## Diagnostics

Immediately before each `session/new` or `session/load`, Ora logs `sending ACP session
configuration` at INFO with the Session id, agent, provider session id, ACP method, selection mode,
server count and the selected plugin ids with package version, configuration revision and transport.
Commands, arguments, environment, URLs, headers and setting values are excluded.

Setup success means the complete list was delivered and accepted, never that the agent finished
connecting: ACP carries no MCP receipt. Agents are expected to treat `mcpServers` as the complete
set for the Session and to connect them before answering. Ora shares one agent process across
Sessions, and OpenCode through 1.18.30 keeps ACP-supplied registrations at process scope, so an
empty list may not remove a server registered by an earlier Session in the same process
([OpenCode #32371](https://github.com/anomalyco/opencode/issues/32371)).

## Host MCP health

Because delivery proves nothing about connection, the Host keeps a third, delivery-independent fact:
**Host MCP health**, established by a bounded MCP handshake the Host performs itself.

- **When.** After an MCP package becomes installed and eligible, after its configuration is saved as
  complete, and after every `session/new` or `session/load`. Also on the user's re-detect action and
  as a backfill for members still unknown. Concurrent triggers for one identity share one probe;
  installing, saving and starting a Session never wait for it.
- **How.** The probe uses the same resolved transport that setup delivers (never a second
  configuration parse) and performs `initialize`, `notifications/initialized` and `tools/list`, then
  tears down; a stdio probe also confirms the child was reclaimed. It has a hard 8-second timeout.
  The client lives in `ora-utils` behind its own feature and is not used during installation, which
  still neither runs commands nor opens connections.
- **Identity.** Plugin id, exact package version, configuration revision and transport, plus the
  absolute Session cwd for members that substitute `{ "context": "workspace" }`. Those members stay
  `Unknown(context_missing)` on the plugin card, which has no Session directory. Uninstall, update,
  configuration change or lost eligibility drops the old result. Nothing is persisted; after restart
  everything is `Unknown(not_probed)`.
- **Outcome.** `Healthy`, `Unhealthy { code }` with one of `mcp_spawn_failed`,
  `mcp_exited_prematurely`, `mcp_handshake_failed`, `mcp_probe_timeout`, `mcp_tools_unavailable`,
  `mcp_http_unreachable`, `mcp_http_unauthorized`, `mcp_http_server_error`, or
  `Unknown { not_probed | context_missing }`. This family is separate from setup errors.

Health never changes delivery: a failed probe does not fail setup, does not remove a member and
never produces a partial list. A healthy probe is not "in effect in the session": the agent still
connects on its own, so the two can disagree either way.

HTTP probes never follow redirects, including same-origin redirects and session DELETE, and ignore
system proxy environment variables. Configure the final, directly reachable endpoint; endpoints
requiring a proxy or redirect will fail. JSON and SSE response bodies are read incrementally up to
1 MiB; stdio counts all received frames and noise against a 1 MiB handshake budget. Oversized input
and redirects report only the stable handshake/tools failure code, never response text or Location.
This transport policy also applies to Desktop probes.

Presentation stays secret-free: the plugin card shows health beside install and configuration
status with a re-detect action; a Session shows a non-blocking banner listing the `Unhealthy`
members of its own effective set; `listMcpHealth` answers either view and `probeMcpHealth` re-runs a
probe; `McpHealthChanged { plugin_id }` tells clients to re-query. After the setup log line, a
structured event pairs the Session with its members' identity, code and probe duration.

## Node sessions

A Node advertising `session_mcp` accepts `AgentSessionSpec.mcp`: an explicit list of plugin id,
exact package version and scalar settings. Omitted or empty means none; ids must be canonical,
unique and sorted. Limits are 16 servers and 64 KiB total compact JSON settings maps. Node resolves
installed packages with `ora-plugin-config` and the shared resolver in `ora-agent-runtime`; it never
discovers MCPs automatically. `InstallPlugins` already accepts MCP packages. Separate workloads
receive immutable package views so stdio commands remain accessible without opening the Node home.

Package and setting checks precede agent launch. Agent capability checks follow ACP `initialize`
and precede `session/new`: non-empty sets require load support, and HTTP needs HTTP MCP support.
Failure sends no `session/new` or MCP values to the agent and ends with `mcp_plugin_missing`,
`mcp_setting_missing`, `mcp_http_capability_missing` or `mcp_load_session_missing`. Structured setup
failures and preflight logs name the plugin id and, when applicable, setting id and transport, never
values; the terminal detail remains the stable code. For MCP setup failures, `AgentSessionEnded` also
carries those optional identifiers to the Controller (`transport` is `stdio` or `http`), omitting unknown
fields and all fields for other terminal results.

Settings remain in memory and the successful ACP request; ledger inputs retain only MCP identities
and versions with empty settings. Duplicate executions cannot change their live values; changing
settings requires a new execution. Node restart interrupts sessions rather than recovering values.
Settings never become JSONL, Thread, logs, errors or input Debug output. Node live refresh, restore
and Host health probes are outside this delivery path.

### Host MCP servers

A Node advertising `session_host_mcp_servers` also accepts `AgentSessionSpec.host_mcp_servers`:
up to four Controller-provided HTTP servers, each a `name` (`ora-` plus lowercase letters, digits and
`-`, 5–64 bytes, strictly increasing), a `url` and `headers`. Omitted and empty both mean none.
They are not plugin MCP: they never join `mcp`, the Effective MCP Set, revisions, the 16-server limit,
`GetSessionMcpState`, refresh, health probes or the setup-window choice, and an empty `mcp` still
injects no plugin MCP. The Node appends them, in input order, after the plugin servers of every ACP
setup of the execution: `session/new`, `session/load`, lifecycle refresh loads and the setups of a
replacement Agent. It never connects to or probes them, and there is no stdio form.

The URL must fully match the decision's grammar (checked character by character, never by
parse-and-reserialize): plaintext `http` only on the literal `127.0.0.1` or `[::1]`, otherwise
`https`; no userinfo, query, fragment, default port, dot segment or lowercase escape. Headers are at
most eight lowercase tokens per server, excluding the HTTP and MCP stack's own headers. All URLs,
header names and values together are at most 8 KiB. `crates/node-protocol/fixtures/host_mcp_servers.json`
holds the shared accept/reject samples a Controller validates against too. Invalid input is a frame
validation failure that closes the control connection before admission.

Non-empty host servers need the Agent to declare HTTP MCP after `initialize`. Plugin checks come
first; when only the host servers need HTTP, the session ends before `session/new` with
`agent_failed` / `host_mcp_http_capability_missing`, naming no server. A replacement Agent is checked
the same way and ends the execution rather than continuing without them.

URLs and headers stay in memory and the ACP request. The ledger keeps only
`host_mcp_server_names`; a repeated start compares names and keeps the values first accepted. Decode
and validation failures log only their category and field. A Node restart interrupts the session
without asking the Controller to resend values.
