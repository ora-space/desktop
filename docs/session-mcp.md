# Session MCP

Ora delivers configured MCP plugins as **Session Runtime Input**, not as Effect Resources and not
as Workspace files. Every ACP `session/new` and `session/load` — `startSession`, the attach a prompt performs, the
rebuild that replaces a provider session Ora could not restore, agent switch, workflow start, and
live refresh — shares one Session Setup snapshot.

## ACP injection

The Effective MCP Set is every currently installed, statically valid MCP plugin whose
configuration is complete. Incomplete plugins are omitted without failing the rest of the set.
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

MCP refresh, Skill Effect mutation, and Agent replacement share one Agent Session Barrier so new
prompts wait for a safe point. They do not share Effect state: MCP never becomes an Effect
Resource, Desired, or readiness signal.

## Security and compatibility

Setting values may exist in the Configuration Store, a short-lived in-memory Snapshot, and the
ACP frame sent to a trusted Agent. They must not enter Effect, SQLite, Workspace files, logs,
errors, UI DTOs, revision digests, or Agent environment variables. Errors name Plugin ID, Setting
ID, transport, and a stable code only.

Ora does not create, modify, or delete `.mcp.json`, OpenCode JSON/JSONC, ownership sidecars, Git
exclude files, or any other Workspace path for MCP. Existing user-authored MCP configuration is
left untouched. There is no runtime migration off the unpublished file-materialization design.
