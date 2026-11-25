# Agent Runtime

English | [中文](agent-runtime.zh.md)

`ora-agent-runtime` (`crates/agent-runtime`) owns every agent connection Ora can reach and one
serialized actor per Ora Session. It speaks ACP to agents supplied by installed plugins and knows
nothing about where it runs: Desktop hosts it through `ora-backend`, and a sandbox Node hosts the
same code for Cloud runs. Recording, switching and history are in [Session history](session-history.md);
MCP delivery is in [Session MCP](session-mcp.md). Crate internals are in
[`crates/agent-runtime/src/README.md`](../../crates/agent-runtime/src/README.md).

## Runtime observations

Hosts with durable command queues can use `prompt_session_with_admission`. The session actor first
checks provider attachment and MCP readiness, then invokes the host's short synchronous commit before
recording or sending the prompt. A failed commit rejects the turn without executing it. Existing
prompt APIs have no commit callback and retain their behavior.

Workspace and workflow conversation views expose read-only run details: the latest live content,
tool or plan observation and its client receipt time, known pending permission requests, active
tools, and recognized provider retry/compaction reports. Missing reports remain unknown. Reloaded
history does not refresh the live observation clock; changing provider or beginning another prompt
clears stale observations. Parent and projected-child clocks and tools are shown separately;
metadata-free child tool deltas retain their known source. Unobserved/replayed tools have an
unconfirmed source rather than being assumed parent work. Observation ages are not timeout
countdowns. Silence does not establish provider health or a child lifecycle. Only requests exposed
by the host are listed; the current runtime auto-handles permissions, and unsupported question forms
do not become pending UI requests.

OpenCode child-session projections and `_subagent/update` extension projections remain visible
and never pause their parent prompt's inactivity window. A `_subagent/update` frame whose outer
`toolCallId` names a parent delegation still `pending` or `in_progress` in the current attempt
additionally rearms a full parent window: the captured CodeAgent parent `Task` stays `pending`
for the whole delegation, so without this renewal a foreground delegation longer than the first
window would be cancelled and re-sent while the subagent continues emitting frames within each
window. Completed, failed, unknown or absent outer delegation ids do not renew, and a retry
clears the previous attempt's delegation observations (assuming parent `toolCallId`s are unique
within a provider session). Metadata-free child tool deltas retain their known source and do not
renew. The parent delegate's own in-progress update still pauses its existing timer, with pause
taking priority over renewal; explicit Wait and cancel/error cleanup remain unchanged.

## Agents come from plugins

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

The private `manager_startup` module owns manager construction and durable session reconciliation; it runs before connection supervisors start.

The private `model_config` module identifies the same provider-reported selector for startup model intents and stale-load rebuilds. Startup applies only offered values, while rebuild reads the reported current value. A report carries no model when it is empty, when two selects make the fallback ambiguous, or when the `Model` category is not a select. Its `model_selector` rule is public so the workflow executor matches a graph's `modelId` against the same selector.

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
  handshake reported (an applied intent outdates options the agent pushed before it), persists the
  Session, opens its history and installs its actor. Until the actor owns it, a guard releases the
  unexposed provider session (`session/delete` when supported, otherwise `session/close`) on any
  error or cancellation, and failed persistence removes the history file and soft-deletes any
  inserted row.
- **Load.** Loading reads Ora's own record and touches no agent: it creates no provider session and
  changes no lifecycle state, so a conversation opens even if its agent is gone. If an actor exists
  it answers its own load (replaying the record and then following a prompt in flight) without
  taking ownership of that prompt. While it is attached, the options and the command catalog its
  provider last reported precede the replay; neither is recorded and both end with the attachment.
- **Attach.** A prompt is what acquires a provider session. A session holding no channel registers a
  route on the current connection and calls `session/load` with its private provider id and the
  current MCP snapshot, discards whatever the agent recites, and marks the row `Running`. Options
  and commands from that handshake reach the client as updates ahead of the turn. An agent usually
  reports its commands only after answering setup, so a command catalog that arrives while no prompt
  runs is kept, newest only, and delivered at the start of the next prompt. Draining an idle session
  discards its other updates, so they do not cross into that prompt; updates that arrive while a
  title poll borrows the session are queued for it instead, again with only the newest catalog, and options as they are by then.
  Admission waits for the attach and the MCP check, so a send that cannot reach an agent fails as a send.
- **Rebuild.** An agent that cannot load the session, or fails to restore it, gets a replacement
  provider session from `session/new` with the same snapshot, and the next prompt carries the
  recorded transcript, exactly as an agent switch does. A load can also report success for a
  session the agent no longer holds: when that channel's first prompt fails without any assistant
  output or tool call, the runtime closes it, rebuilds, publishes the new setup updates, and
  resends the prompt once with the transcript and the model the restored provider reported. If the
  retry fails too, that attempt's error is returned; only a failure to rebuild returns the first
  error. The retry only applies to a reusable `RequestFailed` before any output; a cancelled or
  otherwise finished first prompt consumes it. A load follower registered during that first
  attempt stays with the logical turn across the rebuild, so it receives exactly one terminal
  boundary, and it is told the options and commands of the replacement. Commands the stale session
  had reported are withdrawn until the replacement reports its own, and so are its options when the
  replacement offers none.
- **Configure.** A session that holds a provider reports its own options and is configured through
  `setSessionConfig`. The actor asks the agent itself and is the only holder of the session's options:
  answers and the agent's own `config_option_update`s are taken in the order their frames were read
  from the connection, whether a turn, a title poll, an MCP refresh or session setup reads them, or
  none of these. It takes the answer before the caller gets it, so a view opened later sees the
  options after the change. A caller stops waiting after 30 seconds; a later answer is still taken
  while the provider session lasts, and shown by the turn that is running. An agent that does not read
  holds up the caller, not the actor. Changes in flight do not wait for each other. A change whose
  provider session goes before answering is settled then: it succeeds when the host gave the session
  up (`Stop`, which is also how a switch or a deletion reaches the actor, a turn whose owner stopped
  reading, or a rebuild, which leaves the caller on the replacement's options) and fails when the
  agent was lost. A session that holds no provider is asked directly, and attaching waits for that.
  A model chosen while a session held no provider is an intent applied at the next
  attach. Reasoning effort is read only from the
  `thought_level` option a live session reports; there is no pre-session effort catalog.
- **Stop and delete.** Explicit stop calls `session/close` when advertised, unloads the route and
  keeps provider history. Deletion is described in [Session history](session-history.md#deletion).

The frontend remembers, per agent and in browser storage only, the model last picked for a chat
that had not started, and offers it only while the agent's current catalog still lists it.

### Discovery

Pre-session models and commands come from the plugin's `agent/discover` (or, for older plugins,
`agent/list_models`, which knows no commands), called on demand with the Workspace's working directory
and a 60-second budget. Ora does not cache the result; the plugin owns discovery and any cache, and if
it needs ACP it launches a separate one-shot process through the host's child-process API rather than
using Ora's shared connection. A failure affects only that request; empty lists are valid.

A caller says whether it wants commands. The plugin then waits for the agent's first command report,
at most two seconds after `session/new` answered and never into the time it keeps for cleanup, and
answers without commands when none arrived. Callers that only need models (the workflow editor, a
workflow node's model picker, scheduled-task validation) do not ask, so they never wait. They have
their own frontend cache entry: an answer without commands must not hide them from the chat composer.

The composer shows the commands the session reported; before any report, and while a move onto another
agent is pending, it shows the discovered commands of the agent the next message goes to. Only a
session's own report decides whether a typed command is unsupported, because discovery runs without MCP
servers. Commands are untrusted text: the composer shows them as plain text, at most 500, drops names
that are empty, over 128 bytes or contain whitespace, control or format characters, and cuts
descriptions at 4096 bytes and hints at 256. Delivery is never cut, and a session that goes over 500
commands leaves one warning in Desktop's log.

### Permissions

Ora currently has no user-configurable approval policy. An active actor answers every permission
request immediately through the same path a user's choice would take, preferring `AllowOnce` and
falling back to `AllowAlways` only when no one-time option is offered. A remembered choice is
persisted by the agent itself: Claude writes it into the project's `.claude/settings.local.json`,
which on Node is the user's checkout and would ship with the Revision, and on Desktop would record
an approval the user never made. A request that offers no allow option cancels the turn. A request that arrives while no prompt is active is answered as cancelled.

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
| Plugin discovery | 60 s |
| `session/new` / `session/load` inactivity | 30 s, or 120 s when MCP servers are delivered; rearmed by setup notifications |
| Prompt inactivity windows | 45 s, then 60 s, 90 s and 120 s for up to three retries; paused by running tools and permission waits; subagent frames renew open CodeAgent delegations |
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
inactivity windows. `Wait` sends once and ends a turn after its workflow silence limit (fixed at 30 minutes).
For finite waits, tools and pending permissions keep counting toward the deadline. Only attributable changes renew it. Select `Unbounded` for long silent work. See [Workflow runs](../workflows/workflow-runs.md#prompt-inactivity).

Under `Timeout`, agent messages, thoughts, plans and tool lifecycle updates rearm the window; session chrome
(`available_commands_update`, `current_mode_update`, `config_option_update`,
`session_info_update`, `usage_update`) does not. A running tool pauses the window until the last
parallel tool settles and a permission wait pauses it until answered, so a long tool can run for
hours. There is no absolute prompt limit.

CodeAgent `_subagent/update` frames are the one projected exception: while their outer
`toolCallId` names a delegation still `pending` or `in_progress` in the current attempt, each
frame rearms a full window without pausing it, and a retry starts fresh without the previous
attempt's delegation observations (assuming parent `toolCallId`s are unique within a provider
session; if a provider reused the same id on a new open delegation, a late child frame from the
old task could still renew it). Because the captured CodeAgent parent `Task` never reports
`in_progress`, this keeps a foreground delegation alive only while subagent frames continue to
arrive within each window. Renewal checks frame arrival rather than subagent health, leaving two
accepted limits until CodeAgent reports parent `in_progress`: if a normal single tool inside the
subagent runs longer than the active window without intermediate frames (or the subagent stops
sending frames), the parent prompt still cancels and retries, rerunning the subagent and that
tool; conversely, a subagent that keeps sending frames (even in an error or retry loop) renews
the parent window indefinitely and must be stopped manually.

When a window expires the runtime sends `session/cancel` and waits up to the five-second grace for
the stalled request's response. If it arrived with `stop_reason: cancelled` and the schedule has
windows left, the same user prompt is re-sent on the same provider session, preceded by a
`retrying { retry, maxRetries }` marker on the owning stream. That response fence is required
because ACP updates carry no request id. A missing fence, another stop reason or an exhausted
schedule fails the prompt with `agent_timed_out` and isolates only that Session. A retry never closes
the session or duplicates the recorded user message. Before resending, it records a
`promptRetried` boundary after the cancellation drain. Updates from that drain belong to the
abandoned send. The host refreshes the attempt instruction and completion token for the new send.

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

A host composes the runtime through `AgentRuntimeHost`, whose associated types supply six
interfaces; they are generic parameters, so each host gets its own monomorphized runtime.

| Interface            | Runtime needs                                                      | Desktop                                 | Node                                     |
| -------------------- | ------------------------------------------------------------------ | --------------------------------------- | ---------------------------------------- |
| `SessionStore`       | Create, read, list and update session rows; soft delete            | `SqliteSessionRepository`               | `MemorySessionStore`                     |
| `AgentAttach`        | List agent plugins, attach to a process and its tap, stop it       | `PluginApi`                             | The execution's own one-plugin lifecycle |
| `SessionSetup`       | The Session MCP view, revisions and health observation             | Session MCP resolver and MCP health     | `NoSessionMcp`                           |
| `Skills`             | Current Available Skill catalog entries and readable storage roots | `DesktopSkillCatalog`                   | `NoSkillCatalog`                         |
| `RuntimeEvents`      | Title and model-catalog notifications; every history line written  | `AppEventPublisher`                     | Each line becomes a Thread event         |
| `WorkspaceDirectory` | A Workspace's `cwd`                                                | Workspace repository plus the path base | The execution's checkout                 |

On Desktop, `ora-agent-runtime` injects `<available_skills>` (full catalog on a live session's first
prompt, cold `session/load`, reopened stopped session, or post-compaction reset) and
`<available_skills_update>` (path-keyed deltas on mid-session catalog changes) into outgoing prompts—after any
leading `/command` text block—and forwards the two readable Skill storage roots in ACP
`additionalDirectories` (when advertised) plus `_meta["ora/skillDirectories"]` during `session/new`
and `session/load`. `NoSkillCatalog` leaves Node sessions completely unchanged.

Failures leave the crate as `RuntimeError` (classification, typed public error, context, source
chain); Desktop converts it to `BackendError` and back without loss. The crate must not depend on
`ora-backend`, SQLite or Tauri. Entry points for hosts without a person present
(`wait_for_agent`, `start_session_with_id`, `prompt_session_as_message`) are described in
[Node agent sessions](../node/agent-sessions.md).
