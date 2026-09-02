# plugin_agent

`plugin_agent` turns one installed agent plugin into a supervised connection: every agent Ora
reaches is one of these, there is no other kind of provider. It attaches to a plugin process the
plugin lifecycle already owns, verifies the plugin declared the whole agent contract, brings the
agent up, and exposes the plugin's notification channel as an ACP message stream and sink.

This module is the only place in the agent runtime that knows a plugin exists. Everything above it
sees a `RuntimeConnection` and cannot tell which kind of provider produced it.

## Responsibilities

- Attach to one lifecycle-owned plugin process and read the notifications of that one generation.
- Reject, at handshake time, any plugin whose registration does not cover `agent/start`,
  `agent/stop`, `agent/list_models`, and the emitted `agent/acp`.
- Call `agent/start` and confirm the plugin will speak a protocol version this host understands.
- Read the plugin's pre-session model list through `agent/list_models`, on demand and with the
  Workspace directory the caller resolved. This is not part of bringing a connection up, and it
  carries its own timeout because a plugin may start a one-shot process to answer it.
- Relay ACP messages in both directions as `agent/acp` notifications.
- Ask the plugin to stop its agent before the lifecycle ends the plugin's process tree.
- Convert registered Workspace-relative Skill locators into host-owned Effect
  Resources. MCP is not an Effect Resource: configured MCP plugins are delivered
  through ACP `session/new` and `session/load` `mcpServers`. The canonical Plugin
  ID is the consumer identity; a plugin never chooses that persisted identity.
- Define `effect/coordinate`, `effect/reactivate`, and `effect/verify_ready` as the generic Consumer
  adapter boundary.

## Non-responsibilities

- Owning the plugin process. `ora-plugin-lifecycle` starts, stops, and reports every plugin
  process; this module borrows one and never spawns, kills, or reaps it.
- Discovering, validating, installing, or enabling plugin packages.
- Interpreting ACP. The host is a pipe for `agent/acp` payloads and never parses, validates, or
  rewrites them, which is what lets a plugin support ACP methods this host has never heard of.
- Supervising the agent's process. A plugin spawns and owns its agent CLI itself.
- Retry, backoff, crash-loop detection, and connection state. Those belong to the connection
  supervisor, which treats a dead plugin exactly as it treats a dead CLI.

## Process ownership

An attachment (`PluginApi::attach_agent`) is a `PluginGenerationLease` pinned to one process generation
plus a lossless tap of that generation's notifications, opened through the backend's notification
sink rather than by taking the process stream: the lifecycle's pump stays the only reader of the
process, and a restarted plugin can never leak frames into a connection that belonged to its
predecessor. A connection generation therefore owns its tap but not the process: ending a
generation asks the lifecycle to stop the plugin, which keeps the runtime state the settings
surface reports identical to what the agent runtime is actually talking to, and leaves the next
attach to start a fresh process rather than resuming a half-used one.

## Boundaries and failure semantics

The contract check runs the moment the attachment completes — before any session exists and before
a user is waiting on a prompt — so a plugin that does not implement the contract surfaces as a
failing agent rather than as a failure in the middle of someone's turn. That failure is terminal:
the supervisor publishes `Failing` and abandons the agent for the rest of the process instead of
retrying, because the same plugin will fail identically every time and retrying only produces a
warning per backoff interval.

`agent/start` failures split in three, along the one line that matters here: whether another
attempt could ever produce a different answer. `-32001` means the agent CLI the plugin wraps is
absent from this machine; the user can install it while Ora runs, so that is an expected local
configuration, reported as `agent_not_installed` and retried without logging or contributing to
the crash counter. `-32002` means the CLI the plugin's own package ships cannot run here at all —
a wrong-target, broken, or unrunnable bundled executable — which fails identically on every
attempt; it is terminal, like an unservable contract, so the package fault surfaces once instead
of disappearing behind a quiet missing-CLI report. Every other code is a genuine startup failure.
More than three genuine failures in one minute opens the connection supervisor's restart circuit,
publishes `Failing` to the UI, and stops automatic retries.

A plugin the lifecycle refuses to start — because the user disabled it or uninstalled it — is
reported exactly like a missing CLI, so the supervisor keeps retrying it silently until the user
enables it again.

ACP travels as notifications rather than plugin method calls. ACP frames already carry their own
ids, cancellation, and ordering, so a second correlation layer would mean two timeouts and two
cancellation paths per frame, and the runtime's control-call timeout would sever prompts that
legitimately run for minutes.

A single unusable inbound frame — one that is not an object, or a notification method this runtime
does not consume — is dropped with a warning rather than failing the connection: one bad payload
must not end every live session on that agent. Frames that arrive before `agent/start` returns are
discarded, because they belong to no connection.

Teardown is `agent/stop`, then a lifecycle stop that sends `ora/shutdown` and kills the process
tree once the shutdown timeout expires, and waits until that tree has actually exited before a
replacement generation may start. `agent/stop` has its own short deadline, well inside the host's
cancellation grace, and the plugin is ended whether or not it answered — teardown must never be the
reason shutdown stalls. Plugin cleanup is best effort; final reclamation of the process tree is
always the host's.

The same boundary applies before a connection generation is published: contract verification,
`agent/start`, model discovery, or ACP initialization failure stops the plugin before the
connection supervisor schedules another attempt.

## Effect coordination

An Agent registration may include `effectResources`. Each declaration contains
`workspaceRelativePath`, `materializationFormat`, and either `uninterrupted` or
`quiesce_before_mutation`; it never contains an absolute Workspace path or a persisted identity.
Ora validates the portable locator and maps the canonical Plugin ID to a stable Consumer. Each
local Workspace gets its own Target, while identical physical Resource declarations share one
Resource and merged projection inside that Workspace.

Before a shared Resource mutation, Ora calls `effect/coordinate` for every affected Target whose
binding requires quiescence. The request names the exact `targetId` and complete `resourceIds` set.
The plugin must stop new work that could consume those Resources and return an idempotent proof.
After exact Resource verification, `effect/reactivate` releases that barrier. If reactivation
replaces an Agent instance, Ora detaches its live ACP sessions so the ordinary `session/load` path
re-establishes them before their next prompt.

`effect/verify_ready` receives `targetId`, `generation`, `consumerRevisionId`, and
`projectionDigest`. Its proof advances Target readiness only when all four values match the current
projection. Coordination receipts do not imply readiness for the Target's other Resources.

`ora_backend::effect_worker` drives this protocol from durable, fenced Target and Resource claims.
A disconnected Consumer is already safe to mutate and will read the Resource on its next start, so
the worker records a disconnected adapter receipt without launching the plugin. Failures before a
journal exists enter retry scheduling; failures after preparation enter explicit recovery instead
of invoking a second mutation.

The worker also converges declarations in the opposite direction. Registration pairs a new
Consumer with existing Workspaces immediately, and every worker pass pairs the current declaration
snapshot with Workspaces created later. This level-triggered pairing prevents a one-shot process
event from leaving a Workspace without its Target.

MCP is not projected into Workspace files and is not injected as `ORA_MCP_*`
environment variables. Session MCP setup lives in `session_setup` and is
described in [Session MCP](../../../../../docs/session-mcp.md).

## Sandboxing

Agent plugins currently receive `--allow-run` plus read, env, and network access, because they spawn
and own the agent CLI. The set itself is `ora_plugin_lifecycle::agent_permissions`, applied by the
lifecycle's launcher, which is the only place a plugin process is created. An agent plugin is
therefore roughly as privileged as the host itself. This is a deliberate, documented gap rather
than an oversight: capability narrowing is deferred until the agent contract is proven, and
closing it later changes only how the plugin is started, never the `agent/acp` pipe.
