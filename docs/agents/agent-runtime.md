# Agent Runtime

English | [中文](agent-runtime.zh.md)

`ora-agent-runtime` (`crates/agent-runtime`) owns every agent connection Ora can reach and one
serialized actor per Ora Session. It speaks ACP to agents supplied by installed plugins and knows
nothing about where it runs: Desktop hosts it through `ora-backend`, and a sandbox Node hosts the
same code for Cloud runs. Recording, switching and history are in [Session history](session-history.md);
MCP delivery is in [Session MCP](session-mcp.md). Crate internals are in
[`crates/agent-runtime/src/README.md`](../../crates/agent-runtime/src/README.md).

## Agents come from plugins

Cloud executions with a declared model adapter carry an immutable configuration identity and exact Settings JSON separately from the prompt. Node verifies its management-owned archive receipt, installed declaration, target and protocol before acquiring model authority. The live `agent/start` RPC adds a temporary token; it is never part of the Controller input, ledger or history. An adapter must apply that context and return matching version/fingerprint/revision confirmation before ACP initialization. Older undeclared OpenCode runs retain their explicit compatibility route. Declared plugin/child environments start empty, with only process necessities, private HOME/state, public CA and explicit adapter variables supplied by their owners.

Every agent is supplied by an installed `agent`-kind plugin; nothing is bundled, and Ora ships with
no agents until the user installs packages that supply them (one agent per package). The plugin
owns its agent process end to end: it decides which CLI to run and relays ACP frames to the host
over its `agent/acp` notification channel. See
[`plugin_agent/README.md`](../../crates/agent-runtime/src/plugin_agent/README.md) and
[Plugin system](../plugins/plugin-system.md).

- **Identity.** A Session stores an `AgentRef`: the full canonical plugin id of the supplying
  package (`official/ora-space.claude`). The namespace is part of the identity because two
  marketplace sources may publish the same identifier. An identity no supervisor recognizes means
  "not installed right now", reported as `agent_runtime_unavailable`. Values stored under an older,
  shorter spelling are carried as written and match no agent until the user picks the agent again;
  nothing rewrites them.
- **Supervision.** The supervised set is exactly the installed agent set and is reconciled on every
  install, uninstall and scan, so a newly installed agent needs no restart. A supervisor attaches to
  a running plugin through the plugin lifecycle (`PluginApi::attach_agent`, which starts it on
  demand), reads that generation's `agent/acp` notifications through a lossless tap, and asks the
  lifecycle to stop the plugin when a generation ends.
- **Replacement.** A supervisor handle belongs to one installed incarnation: an update or reinstall
  retires the old supervisor, and a handle cloned earlier does not follow the swap in the shared
  registry. When a Session reacquires its provider channel, its actor resolves the current
  supervisor by the persisted `agent_ref` and uses it for that attach and its MCP check. Existing
  conversations therefore resume through the new supervisor without restarting Ora or stopping the
  Session, while a removed plugin keeps reporting `agent_runtime_unavailable`. Restore and
  transcript-backed rebuild acquire their channel through the same path.
- **Executables.** The host owns OS processes: `ora/childprocess/spawn` accepts a `command` the OS
  resolves or a `packageCommand` the host joins onto the plugin's install root. An agent package is
  published either with a bundled CLI (per-target releases) or without one (a universal release that
  finds the user's install on `PATH`); the plugin asks for its bundled path first and falls back to
  `PATH` when the host answers `package_command_missing`.
- **Failure classes.** A CLI absent from the machine reports `agent_not_installed`, stays retryable
  and is not logged per attempt. A bundled executable that will not run reports `agent_unusable` and
  that agent is abandoned for the rest of the process.

## Connections and supervision

At Backend start, stale `Running` session rows are reconciled to `Stopped`, then one dedicated
runtime thread per agent starts it and performs `initialize`. The handshake advertises the
session-config-option capability (the model selector depends on it). Each agent retries
independently with exponential backoff from 250 ms to a 30 s cap; Ora stays usable even if every
agent fails, and operations on an unavailable agent report `agent_runtime_unavailable`. More than
three genuine failures within one minute opens that agent's restart circuit: it publishes `Failing`
and stops retrying for the rest of the process. A plugin that does not implement the agent contract
enters `Failing` at once.

Connection loss fails that agent's in-flight operations, marks only its registered Sessions
`Stopped`, asks the lifecycle to stop the old process and only then starts a replacement. Sessions
reattach on demand; prompts are never replayed automatically. Dropping the last Backend owner stops
every supervisor, cancels routed operations and terminates each process tree.

The frontend agent catalog is derived at runtime: pickers list installed `agent` plugins with each
package's own display name and logo, offering only identities whose status is `Ready` or
`Starting`. `Unavailable` is polled slowly because it can resolve itself; `Failing` is not.

## Session lifecycle

Opening or navigating to a chat creates no backend state. The frontend keeps the optimistic first
turn locally until `startSession` returns.

- **Start.** `startSession` is the only creation path. It calls `session/new` with the current
  Session MCP snapshot, applies the caller's optional model intent against the options that
  handshake reported, persists the Session, opens its history and installs its actor. Until the
  actor owns it, a guard releases the unexposed provider session (`session/delete` when supported,
  otherwise `session/close`) on any error or cancellation, and failed persistence removes the
  history file and soft-deletes any inserted row.
- **Load.** Loading reads Ora's own record and touches no agent: it creates no provider session and
  changes no lifecycle state, so a conversation opens even if its agent is gone. If an actor exists
  it answers its own load (replaying the record and then following a prompt in flight) without
  taking ownership of that prompt.
- **Attach.** A prompt is what acquires a provider session. A session holding no channel registers a
  route on the current connection and calls `session/load` with its private provider id and the
  current MCP snapshot, discards whatever the agent recites, and marks the row `Running`. Options
  and commands from that handshake reach the client as updates ahead of the turn. Admission waits
  for the attach and the MCP check, so a send that cannot reach an agent fails as a send.
- **Rebuild.** An agent that cannot load the session, or fails to restore it, gets a replacement
  provider session from `session/new` with the same snapshot, and the next prompt carries the
  recorded transcript, exactly as an agent switch does.
- **Configure.** A session that holds a provider reports its own options and is configured through
  `setSessionConfig`. A model chosen while a session held no provider is an intent applied at the
  next attach. Reasoning effort is read only from the `thought_level` option a live session reports;
  there is no pre-session effort catalog.
- **Stop and delete.** Explicit stop calls `session/close` when advertised, unloads the route and
  keeps provider history. Deletion is described in [Session history](session-history.md#deletion).

The frontend remembers, per agent and in browser storage only, the model last picked for a chat
that had not started, and offers it only while the agent's current catalog still lists it.

### Model discovery

Pre-session models come from the plugin's `agent/list_models`, called on demand with the
Workspace's working directory and a 60-second budget. Ora does not cache the result; the plugin owns
discovery and any cache, and if it needs ACP to discover models it launches a separate one-shot
process through the host's child-process API rather than using Ora's shared connection. A failure
affects only that request; empty lists are valid.

### Permissions

Ora currently has no user-configurable approval policy. An active actor answers every permission
request immediately through the same path a user's choice would take, preferring the option that
remembers the choice (`AllowAlways`) and falling back to `AllowOnce`. A request that offers no allow
option cancels the turn. A request that arrives while no prompt is active is answered as cancelled.

### First title

A newly attached Session has a runtime-only title window. Valid `session_info_update` titles are
accepted immediately. After the first prompt ends with `EndTurn`, `MaxTokens` or `MaxTurnRequests`,
an agent that advertises `session/list` is asked at three and ten seconds (each request only while
the actor is idle, with a five-second timeout); otherwise the window closes at ten seconds. The last
valid title that persists wins; duplicates write nothing. A user rename locks the window. Restored
sessions do not reopen it. After a title-only write the actor publishes `SessionTitleUpdated`, a
hint for clients to refetch.

## Flow control

ACP stdout is newline-delimited JSON-RPC. The connection reader hands frames, unbounded and in
order, to a central router; each registered Session owns a bounded 256-item FIFO for updates,
permission requests and terminating responses. Connection loss and queue overflow travel on a
separate control channel, so a terminal failure cannot hide behind a full FIFO. One Session's
overflow stops only that Session.

- While `session/new` has not revealed its provider id, notifications are held in a shared buffer
  of at most 256 entries; registration drains the matching ones and the rest are dropped when the
  last concurrent setup ends.
- Routes are bound to a connection generation; updates from an old connection or an unloaded
  Session are discarded as stale.
- Unknown agent-originated requests get a `-32601` response without ending the connection;
  malformed frames, unmatched responses, oversized frames and stdio loss are connection failures.
- A load or prompt completes only after consuming its matching response, which keeps every earlier
  update in that operation.
- History replay is the one stream that applies backpressure instead of failing fast.

Dropping a frontend iterator or closing a Tauri stream sends `session/cancel`. Cancellation is
session-scoped: it reaches the prompt's owner even from a view that only follows it, records a
cancelled turn and keeps the actor reusable.

## Limits

| Bound | Value |
|---|---|
| `initialize` | 15 s |
| Plugin model discovery | 60 s |
| `session/new` / `session/load` inactivity | 30 s, or 120 s when MCP servers are delivered; rearmed by setup notifications |
| Prompt inactivity windows | 45 s, then 60 s, 90 s and 120 s for up to three retries; paused by running tools and permission waits |
| Cancellation settlement grace | 5 s |
| Connection retry backoff | 250 ms doubling to 30 s |
| Restart circuit | Opens after more than 3 failures in 1 minute |
| Session queue depth | 256 items |
| Structured prompt size | 16 MiB of serialized JSON |
| Handoff transcript size | Unbounded |

### Setup inactivity

Setup waits on inactivity, not a total budget. A request that delivers MCP servers gets 120 seconds
because the agent connects them before answering and real MCP initialization can take tens of
seconds; a request without servers keeps 30. Notifications emitted while a create waits for its id
rearm the deadline, so an agent that reports progress is never cut off and a silent one fails with
`agent_timed_out`.

### Prompt inactivity and retries

Each prompt has a `PromptInactivityPolicy`. `Timeout` (ordinary chat and the default) applies the
inactivity windows; `Wait` never cancels, re-sends or fails a prompt only because it is silent.
Workflow Agent nodes can choose `Wait`, see [Workflow runs](../workflows/workflow-runs.md#prompt-inactivity).

Agent messages, thoughts, plans and tool lifecycle updates rearm the window; session chrome
(`available_commands_update`, `current_mode_update`, `config_option_update`,
`session_info_update`, `usage_update`) does not. A running tool pauses the window until the last
parallel tool settles and a permission wait pauses it until answered, so a long tool can run for
hours. There is no absolute prompt limit.

When a window expires the runtime sends `session/cancel` and waits up to the five-second grace for
the stalled request's response. If it arrived with `stop_reason: cancelled` and the schedule has
windows left, the same prompt blocks are re-sent on the same provider session, preceded by a
`retrying { retry, maxRetries }` marker on the owning stream. That response fence is required
because ACP updates carry no request id. A missing fence, another stop reason or an exhausted
schedule fails the prompt with `agent_timed_out` and isolates only that Session. A retry never closes
the session and never re-records the prompt; output produced before the stall stays, so an agent may
repeat itself.

`Wait` still ends on normal responses, provider errors, connection loss, overflow, explicit
cancellation, owner cleanup and shutdown, and setup and configuration deadlines still apply. It
cannot tell a healthy hidden subagent from a stuck provider, so the user may need to stop it.

## Errors

Every failure reaches the client as a typed public error chosen where it is raised. A process that
will not start reports `agent_start_failed`; every runtime deadline reports `agent_timed_out`; model
listing failures report `agent_model_discovery_failed`. These classify as `Unprocessable` because a
missing, unusable or slow CLI is a local condition the user can act on. Queue overflow and ACP
violations stay `internal_error` because they are Ora defects whose request id must be reportable.
Plugin-authored startup text never enters public error parameters; it reaches logs only.

Prompts are ordered ACP `ContentBlock` values (text, images, audio, resource links, embedded
resources). An empty list or only blank text is rejected.

## Hosts

Hosts may opt into `RuntimeEvents::record_preview` to observe assembled pending records before
settlement. Notifications are capped at ten per second, and a turn boundary sends an empty
replacement. The callback is best effort, produces no acknowledgement and does not change the
`record_settled` durability order. Hosts that do not opt in incur no pending-snapshot copy.

A host composes the runtime through `AgentRuntimeHost`, whose associated types supply five
interfaces; they are generic parameters, so each host gets its own monomorphized runtime.

| Interface | Runtime needs | Desktop | Node |
|---|---|---|---|
| `SessionStore` | Create, read, list and update session rows; soft delete | `SqliteSessionRepository` | `MemorySessionStore` |
| `AgentAttach` | List agent plugins, attach to a process and its tap, stop it, record its Effect consumer | `PluginApi` | The execution's own one-plugin lifecycle |
| `SessionSetup` | The Session MCP view, revisions and health observation | Session MCP resolver and MCP health | `NoSessionMcp` |
| `RuntimeEvents` | Title and model-catalog notifications; every history line written | `AppEventPublisher` | Each line becomes a Thread event |
| `WorkspaceDirectory` | A Workspace's `cwd` | Workspace repository plus the path base | The execution's checkout |

Failures leave the crate as `RuntimeError` (classification, typed public error, context, source
chain); Desktop converts it to `BackendError` and back without loss. The crate must not depend on
`ora-backend`, SQLite or Tauri. Entry points for hosts without a person present
(`wait_for_agent`, `start_session_with_id`, `prompt_session_as_message`) are described in
[Node agent sessions](../node/agent-sessions.md).
