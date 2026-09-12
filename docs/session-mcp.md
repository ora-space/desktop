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

A workflow Session retains its node-local selection through creation, restore, provider rebuild,
and live refresh. After an actor is recreated, its selection comes from the existing node-run
Session relationship and the run's published snapshot. Missing or invalid associated execution
metadata fails closed rather than reverting to automatic discovery. Editing a draft cannot change
an existing run's selection. Plugin package versions and configuration values remain live inputs;
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

## Security and compatibility

Setting values may exist in the Configuration Store, a short-lived in-memory Snapshot, and the
ACP frame sent to a trusted Agent. They must not enter Effect, SQLite, Workspace files, logs,
errors, UI DTOs, revision digests, or Agent environment variables. Logs may contain only the
secret-free revision identity described above. Errors name Plugin ID, Setting ID, transport, and a
stable code only.

Ora does not create, modify, or delete `.mcp.json`, OpenCode JSON/JSONC, ownership sidecars, Git
exclude files, or any other Workspace path for MCP. Existing user-authored MCP configuration is
left untouched. There is no runtime migration off the unpublished file-materialization design.
