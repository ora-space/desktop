# plugin_agent

`plugin_agent` turns one installed agent plugin into something the connection supervisor can treat
exactly like a built-in CLI. It launches the plugin process, verifies the plugin declared the whole
agent contract, brings the agent up, and exposes the plugin's notification channel as an ACP
message stream and sink.

This module is the only place in the agent runtime that knows a plugin exists. Everything above it
sees a `RuntimeConnection` and cannot tell which kind of provider produced it.

## Responsibilities

- Launch one plugin process per agent with the Deno permissions an agent plugin requires.
- Reject, at handshake time, any plugin whose registration does not cover `agent/start`,
  `agent/stop`, `agent/listModels`, and the emitted `agent/acp`.
- Call `agent/start` and confirm the plugin will speak a protocol version this host understands.
- Read the plugin's pre-session model list through `agent/listModels`.
- Relay ACP messages in both directions as `agent/acp` notifications.
- Ask the plugin to stop its agent before the host reaps the plugin's process tree.

## Non-responsibilities

- Discovering, validating, installing, or enabling plugin packages. The caller supplies an
  already-resolved `PluginAgentSpec`.
- Interpreting ACP. The host is a pipe for `agent/acp` payloads and never parses, validates, or
  rewrites them, which is what lets a plugin support ACP methods this host has never heard of.
- Supervising the agent's process. A plugin spawns and owns its agent CLI itself.
- Retry, backoff, crash-loop detection, and connection state. Those belong to the connection
  supervisor, which treats a dead plugin exactly as it treats a dead CLI.

## Boundaries and failure semantics

The contract check runs the moment the handshake completes — before any session exists and before a
user is waiting on a prompt — so a plugin that does not implement the contract surfaces as an
failing agent rather than as a failure in the middle of someone's turn. That failure is terminal:
the supervisor publishes `Failing` and abandons the agent for the rest of the process instead of retrying, because
the same plugin will fail identically every time and retrying only produces a warning per backoff
interval.

`agent/start` failures split in two. `-32001` means the agent CLI is absent from this machine; that
is an expected local configuration, so it maps onto the same public error a missing built-in CLI
produces and is retried without logging or contributing to the crash counter. Every other code is a
genuine startup failure. More than three genuine failures in one minute opens the connection
supervisor's restart circuit, publishes `Failing` to the UI, and stops automatic retries.

ACP travels as notifications rather than plugin method calls. ACP frames already carry their own
ids, cancellation, and ordering, so a second correlation layer would mean two timeouts and two
cancellation paths per frame, and the runtime's control-call timeout would sever prompts that
legitimately run for minutes.

A single unusable inbound frame — one that is not an object, or a notification method this runtime
does not consume — is dropped with a warning rather than failing the connection: one bad payload
must not end every live session on that agent. Frames that arrive before `agent/start` returns are
discarded, because they belong to no connection.

Teardown is `agent/stop`, then an explicit plugin shutdown that sends `ora/shutdown` and kills the
process tree once the shutdown timeout expires, then waits until that tree has actually exited
before a replacement generation may start. `agent/stop` has its own short deadline, well inside
the host's cancellation grace, and the plugin is ended whether or not it answered — teardown must
never be the reason shutdown stalls. Ending the plugin explicitly also matters because plugin
handles are cloned into the ACP transport that live sessions hold: waiting for the last clone to
drop would let one surviving session actor keep a failed plugin running. Plugin cleanup is best
effort; final reclamation of the process tree is always the host's.

The same explicit shutdown-and-wait boundary applies before a connection generation is published:
contract verification, `agent/start`, model discovery, or ACP initialization failure reaps the
partially started plugin before the connection supervisor schedules another attempt.

## Sandboxing

Agent plugins currently receive `--allow-run` plus read, env, and network access, because they spawn
and own the agent CLI. An agent plugin is therefore roughly as privileged as the host itself. This
is a deliberate, documented gap rather than an oversight: capability narrowing is deferred until the
agent contract is proven, and closing it later changes only how the agent is started, never the
`agent/acp` pipe.
