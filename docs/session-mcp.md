# Session MCP

English | [中文](session-mcp.zh.md)

Ora delivers configured MCP plugins as **Session Runtime Input**, not as Effect Resources and not
as Workspace files. Every ACP `session/new` and `session/load` — `startSession`, the attach a prompt performs, the
rebuild that replaces a provider session Ora could not restore, agent switch, workflow start, and
live refresh — shares one Session Setup snapshot.

## ACP injection

Ordinary chats automatically select every currently installed, statically valid MCP plugin whose
configuration is complete. Incomplete plugins are omitted without failing the rest of that set.
Workflow Agent nodes instead use their frozen `mcps` bindings as an explicit allowlist: only
`enabled: true` IDs are delivered, and an empty list delivers no MCP servers. Missing, invalid,
or incompletely configured selected plugins fail setup; unselected plugins are filtered before
configuration and capability checks. Old graphs without `mcps` use an empty allowlist.
Server names are canonical Plugin IDs (`<namespace>/<identifier>`), sorted by that ID. A snapshot
is all-or-nothing: the runtime never sends a partial `mcpServers` list.

Stdio maps to ACP `McpServer::Stdio`; the command is re-checked as an ordinary file inside the
current package version. HTTP maps to `McpServer::Http` and requires the Agent to advertise HTTP
MCP capability. `{ "context": "workspace" }` becomes the Session's absolute cwd; a literal `"."`
stays `"."`. Env and headers use ACP name/value lists.

A non-empty set requires `session/load`, because that is the only frame that can carry a changed
set to a Session already running. If the Agent cannot load sessions, setup fails before any frame
is sent, and that includes the `session/new` Ora uses to rebuild a provider session it could not
restore: a rebuild carries the real Snapshot or it does not happen. MCP is never approximated by
an Agent's own replay or by the transcript Ora injects after a rebuild.

## Live refresh

Live Sessions keep Desired and Active MCP revisions in memory only. Plugin install, update,
uninstall, and Settings save/clear/recover send a secret-free wakeup. Idle Sessions `session/load`
immediately; a busy Session refreshes after the current prompt. Refresh blocks new prompts.
Success advances Active revision; a newer Desired that arrives during load stays pending. Failure
blocks only that Session, and the next prompt retries instead of using the old configuration.
Stopped Sessions do not refresh in the background.

A workflow Session persists its node-local selection on the Session row at creation and retains it
through restore, provider rebuild, Agent switching, and live refresh. Recovery reads that owned
value directly; it never reconstructs authority from a node-run relationship, so missing or
orphaned workflow metadata cannot widen an explicit selection to automatic discovery. Migration
`0011` gives all existing Sessions an empty explicit selection without consulting workflow metadata,
since MCP authorization has not yet been used by users. Existing ordinary chats therefore do not
automatically discover MCPs either. New ordinary Sessions explicitly select automatic discovery;
new workflow Sessions persist their node's explicit selection. Editing a draft cannot change an
existing run's selection.
Plugin package versions and configuration values remain live inputs;
changes outside the allowlist do not change that Session's Desired revision. The editor switches
configure later runs; they are not controls for changing a running Session.

MCP refresh, Skill Effect mutation, and Agent replacement share one Agent Session Barrier so new
prompts wait for a safe point. They do not share Effect state: MCP never becomes an Effect
Resource, Desired, or readiness signal.

## Diagnostics and agent conformance

Immediately before each ACP `session/new` or `session/load`, Ora emits an INFO event named
`sending ACP session configuration`. It includes the Ora Session ID, Agent, provider Session ID
when one exists, ACP method, selection mode, server count, and the selected Plugin IDs with package
version, configuration revision, and transport. It deliberately excludes commands, arguments,
environment variables, HTTP URLs, headers, and Setting values.

Agent adapters are expected to treat the supplied `mcpServers` list as the complete set for that
Session. Ora keeps the shared Agent process model and does not create one OpenCode process per
Session. OpenCode through version 1.18.30 retains ACP-supplied MCP registrations at process scope,
so an empty list is delivered correctly but may not remove a server registered by an earlier
Session in the same OpenCode process. This provider conformance gap is tracked upstream in
[OpenCode issue #32371](https://github.com/anomalyco/opencode/issues/32371).

Setup waits are inactivity windows that widen with delivery, because connecting the delivered
servers is the slow part of a conforming setup: a `session/new` or `session/load` that carries MCP
servers waits up to 120 seconds of silence instead of 30, and setup notifications the agent emits
meanwhile rearm the window (see [ACP Agent Runtime](agent-runtime.md)).

ACP 1.6.0 provides no receipt for MCP connections — `NewSessionResponse` and `LoadSessionResponse`
carry no MCP status, and `SessionUpdate` has no MCP variant — so setup success means the complete
list was delivered and accepted, never that the agent finished connecting. The protocol's
documented session-setup sequence has the agent connect the delivered servers _before_ answering;
an agent that answers first and connects in the background can serve a prompt that arrives before
those connections complete, with no host-visible signal. That window is an agent conformance
responsibility: Gemini CLI fixed the identical race by making prompt handling wait for MCP
initialization ([gemini-cli #18893](https://github.com/google-gemini/gemini-cli/issues/18893), fixed in
[#20205](https://github.com/google-gemini/gemini-cli/pull/20205)), and Claude Code tracks the same
class of first-turn tool race in
[claude-code #83555](https://github.com/anthropics/claude-code/issues/83555). Ora observes Host-side
connection health itself, in memory and without changing delivery semantics; see
[Runtime health](#runtime-health).

## Security and compatibility

Setting values may exist in the Configuration Store, a short-lived in-memory Snapshot, and the
ACP frame sent to a trusted Agent. They must not enter Effect, SQLite, Workspace files, logs,
errors, UI DTOs, revision digests, or Agent environment variables. Logs may contain only the
secret-free revision identity described above. Errors name Plugin ID, Setting ID, transport, and a
stable code only.

Ora does not create, modify, or delete `.mcp.json`, OpenCode JSON/JSONC, ownership sidecars, Git
exclude files, or any other Workspace path for MCP. Existing user-authored MCP configuration is
left untouched. There is no runtime migration off the unpublished file-materialization design.
Installing an MCP therefore adds it to the global catalog only; a workflow node's explicit
selection is the Session-level authorization decision.

## Runtime health

Setup success means the Host sent a complete `mcpServers` list. It does not mean the Agent
connected to those servers, listed their tools, or made them visible to the model: ACP 1.6.0
carries no receipt for MCP connections, and an Agent that skips or fails a server while still
answering `session/new` is compliant. Ora therefore keeps a third, delivery-independent fact of its
own — **Host MCP health** — established by a bounded MCP client handshake the Host performs itself.

After an MCP package is installed and eligible, after its configuration is saved as `Complete`, and
after every `session/new` or `session/load`, the Host runs one handshake against the same binding
product Session setup delivers: the output of the same `resolve_mcp_transport` used to build the
ACP payload, never a second configuration parse. The handshake performs `initialize`,
`notifications/initialized`, and `tools/list`, then tears the connection down; a stdio probe also
closes the child's stdin and confirms the process was reclaimed. Its hard timeout is 8 seconds,
well below the session-setup budget, so a probe cannot impersonate setup.

Health is a separate channel and never changes delivery. A failed probe does not fail `session/new`
or `session/load`, does not remove the member from the Effective MCP Set, and never produces a
partial `mcpServers` list; resolution failures still fail the whole setup as described above.
Installing, saving, and starting a Session never wait for a probe. Concurrent triggers for one
identity share a single probe, a save probes once without retrying on its own, and the only other
probe sources are the user's "re-detect" action and the Session backfill for members that are still
unknown.

An entry's identity is the canonical Plugin ID, the exact package version, the configuration
revision, and the transport. Members whose arguments substitute `{ "context": "workspace" }` also
bind the absolute Session `cwd`. The plugin card has no Session directory, so those members stay
`Unknown(context_missing)` there and are never probed against an invented path; a Session result is
never written back to the card. Uninstall, an update to a new version, a configuration revision
change, or lost eligibility drops the old identity's result immediately. Nothing is persisted and
there is no TTL or background re-check, so after Ora restarts every identity starts from
`Unknown(not_probed)` again.

An outcome is `Healthy`, `Unhealthy { code }`, or `Unknown { reason }`. `reason` is only
`not_probed` or `context_missing`, and `code` is one of `mcp_spawn_failed`,
`mcp_exited_prematurely`, `mcp_handshake_failed`, `mcp_probe_timeout`, `mcp_tools_unavailable`,
`mcp_http_unreachable`, `mcp_http_unauthorized`, or `mcp_http_server_error`. The family is closed
and deliberately separate from the setup codes (`mcp_setting_missing`,
`mcp_http_capability_missing`, …): a probe result is a runtime observation, not a delivery error.

**A successful Host probe is not "in effect in the session".** It says the Host completed a
handshake at that moment; it is not `MCP Ready`, it is not an Active revision, and it does not mean
the Agent connected the server or made its tools visible to the model. The Agent still connects and
runs the servers on its own, so a probe and a Session can disagree in either direction.

Presentation stays secret-free and independent of the other facts:

- The plugin card shows health as a third status line beside install state and configuration
  completeness, with the stable code and a re-detect action. A configuration-incomplete or
  ineligible plugin is not probed and shows no such line.
- A non-blocking banner in a Session lists the `Unhealthy` members of that Session's Effective MCP
  Set — the automatically discovered set for a chat, the frozen whitelist for a workflow — and links
  into plugin configuration. An unselected MCP never appears there, an explicit empty selection
  shows no banner, and `Unknown` is not a failure and never blocks a prompt.
- `listMcpHealth` answers the card view (no `cwd`) or one Session view (absolute `cwd`), returning
  identity, status, and stable code only — never Setting values, credentials, argv, env, headers, or
  third-party response text. `AppEvent::McpHealthChanged { plugin_id }` tells clients to re-query;
  it carries no status of its own.
- After the existing INFO event `sending ACP session configuration`, a structured probe-result event
  pairs each Session with its members' identity, stable code, and probe duration. It records what
  the Host observed; it does not claim the Agent did the same.

The MCP client used for probing lives in `ora-utils` with no Ora domain vocabulary, behind its own
Cargo feature, and is deliberately absent from the plugin-manager install-verification path:
installation still neither executes a command nor opens a connection to decide whether a package is
valid.
