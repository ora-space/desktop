# Node Agent Session Execution

English | [中文](agent-session.zh.md)

A Node runs Agent session executions with
[`ora-agent-runtime`](../agent-runtime.md#runtime-crate-and-hosts): in the
checkout a clone execution left, it starts only the agent plugin the execution
names, at its exact version, writes every settled record to the session JSONL
before handing it to the Node ledger as a Thread event, and runs session
commands in acceptance order. The decisions behind it are the
[agent-runtime root decision](../../specs/decisions/node/agent-runtime/0-shared-agent-runtime-crate-hosted-by-node.md)
and the
[protocol follow-up decision](../../specs/decisions/node/protocol/20260928-streamed-thread-events-and-session-commands.md).

The code lives in `apps/ora-node/src/session` and builds on Linux only. Ledger
tables, protocol message wiring, and plugin installation are not here; they meet
session execution only through the interfaces below.

## Interfaces

| Trait                  | Provided by       | What session execution uses it for                                                                                        |
| ---------------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `SessionLedger`        | Node ledger       | Append Thread events, write the terminal result, read queued commands (all of them, in acceptance order), settle commands |
| `CheckoutResolver`     | Clone bookkeeping | Resolve a clone execution ID to its checkout; a session never composes the path itself                                    |
| `PluginCatalog`        | Plugin installer  | Take a use lease; look up the directory of an exact installed version                                                     |
| `PriorRevisionRestore` | Revision restore  | Restore the prior Revision a resumed session names into its checkout before the agent starts                              |
| `SessionHost`          | Session execution | Implemented by `AgentSessions`: `start`, `command_arrived`, `recover_interrupted`, `sealed_history`                       |

`queued_commands` returns the whole queue rather than its head: while a turn
runs, an `EndSession` accepted behind queued user turns must be visible so the
turn can be cancelled at once and the turns ahead of the end discarded.

## One Runtime Per Execution

Each session execution composes its own plugin lifecycle and agent runtime. The
plugin root is still `plugins/` in the Node data directory, and session
histories live under `sessions/` beside it, but this lifecycle reports and
starts only the execution's agent plugin, launching it with the execution's Git
identity. "Only the named plugin" and "the identity reaches only this
execution's process tree" therefore hold by construction rather than by
filtering a shared instance.

| Runtime interface    | Node implementation                                                                                   |
| -------------------- | ----------------------------------------------------------------------------------------------------- |
| `SessionStore`       | `MemorySessionStore`: a Node never resumes a session after restarting, so rows need not outlive it    |
| `AgentAttach`        | The execution's own plugin lifecycle, with one plugin; no Effect consumer (a Node projects no Skills) |
| `SessionSetup`       | `NoSessionMcp`: MCP configuration and secrets have no delivery path yet                               |
| `RuntimeEvents`      | Every history line becomes a Thread event; title and model catalog events are dropped                 |
| `WorkspaceDirectory` | The checkout                                                                                          |

The Git identity is exported as `GIT_AUTHOR_*`/`GIT_COMMITTER_*` both on the
plugin process, which the processes it spawns directly inherit, and on every
process the host spawns for the plugin through `ora/childprocess/spawn`, which
inherits the Node's environment instead. The Node writes no Git configuration.
These variables are layered on top of the inherited environment, so model
credentials and proxy settings from the Node's environment still reach the
agent.

### Separate workload user

When the deployment runs Git workloads as a separate user (`process.workload_uid`),
agents run as that user too: the Deno plugin process and every process it asks
the host to spawn drop to the workload UID (group equal to it, no supplementary
groups, no capabilities, `no_new_privs`, umask `077`) before executing, and a
spawn that cannot drop fails instead of running as the Node. The agent can then
commit in the checkout the workload user owns, and cannot read the Node's data
directory.

Each session gets `<agent.workload_directory>/<sha256(execution_id)>/` (root,
`0711`) holding:

- `package/`: a hard-link view of the installed package in fresh `0755`
  directories (copied with `0644`/`0755` modes across filesystems; links and
  special files refuse the view). The plugin is launched from the view; the
  lifecycle still discovers and verifies the installed package.
- `home/`: `0700`, owned by the workload user. `HOME`, `XDG_CONFIG_HOME`,
  `XDG_DATA_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`, `DENO_DIR` (under the
  cache) and `DENO_NO_UPDATE_CHECK=1` point there, on the plugin and on
  host-spawned processes alike.

Before launch the checkout is handed back to the workload user without
following links, for checkouts a previous Node's root agent wrote in. The
session directory is removed once the plugin stopped; opening the service
removes every leftover one after interrupted sessions are settled. A failure to
prepare ends the session as `agent_failed{agent_start_failed}`.

## How an Execution Runs

1. `start` returns at once; the session resolves the checkout in the background
   and ends as `agent_failed{checkout_unavailable}` without one.
2. It takes the plugin lease before looking up the version. When the catalog
   lacks that version, or the package the plugin root holds is not in that
   version's directory or is not an agent, the session ends as
   `agent_failed{agent_plugin_unavailable}` with no plugin process ever started.
   The lease is held until the plugin's whole process tree has exited.
3. When the input names a `prior_revision`, the session restores it into the checkout before the
   checkout is handed to the workload user and before any plugin starts (see
   [Revision delivery](revision-delivery.md#resuming-a-prior-revision)). A failed restore ends the
   session as `agent_failed{prior_revision_unavailable}` or
   `agent_failed{prior_revision_base_unavailable}` with no plugin process ever started; a restore
   onto rewritten remote history appends one fixed English note to `initial_turn`.
4. It waits for the agent connection to be ready (bounded; a timeout or a
   supervisor that gives up ends as `agent_failed{agent_unavailable}`), creates
   the session with the execution ID as its Ora Session ID
   (`agent_failed{agent_start_failed}` on failure), and sends `initial_turn`.
5. Every history line settled while a turn runs becomes a Thread event carrying
   that turn's `turn_id`; the user message also carries the same identity in the
   JSONL as its ACP `messageId`. A record over 256 KiB keeps only `at`, `seq`,
   and `type` in the Thread and is marked `truncated`; the JSONL keeps the
   original.
6. `command_arrived` wakes the session to read its queue. A `SubmitUserTurn`
   runs after the current turn, in acceptance order, and is settled `executed`
   when it starts; repeated wakes never run it twice. An `EndSession` settles
   the user turns accepted before it as `discarded`, stops the session
   (cancelling a running turn and recording `TurnEnded{cancelled}`, with
   `session/close` when supported), then settles itself `executed`.
7. At the end the session stops, releases its runtime (whose connection
   supervisor then stops reconnecting), stops the plugin and waits for its whole
   tree to exit, releases the lease, settles any still-queued command as
   `discarded`, writes the terminal result, and then forgets the live session.
   Service shutdown waits for that final write; delivery never sees an ended session
   whose history is still being written.

A failed or timed-out agent turn only records `TurnEnded` and the session
continues. A turn that cannot be admitted at all, because the agent cannot be
reached, ends the session as `agent_failed{agent_unavailable}`.

## Record Order and Crashes

The runtime writes the history one line at a time and calls `record_settled`
synchronously after a line is in the file and before the next is written; the
Node does `append_thread_event` there. Every Thread record is therefore in the
JSONL, in the same order, and a crash between the two writes leaves the JSONL at
most one line ahead of the Thread.

Once `append_thread_event` fails, the mirror stops for good — later lines never
reach the Thread, which would otherwise have a hole nothing explains — and the
session ends as `agent_failed{thread_unavailable}`.

After a Node restart, `recover_interrupted` ends every session execution without
a terminal result as `interrupted`: the history's only writer ended with the old
process, so the file is final. `sealed_history` answers `history_unavailable`
while the session is still running in this process and otherwise returns the
JSONL at the path `ora-history` defines.

## Known Gap

The plugin process and the agent CLI are started by the Node directly, as
Desktop does (process group, whole-tree termination), and the tree is cleaned up
when the session ends or the Node stops normally. They are not yet inside the
host/guardian execution process scope, because the guardian does not yet provide
the stdin and protocol streams a plugin needs. When the Node crashes, the plugin
exits on its closed stdio, but a descendant that ignores that is not reclaimed
until a new process I/O decision covers plugins.

## Tests

`apps/ora-node/tests/agent_session.rs` drives a real echo agent plugin process
(`ora-node-echo-agent`, standing in for `deno` the way the E2E `fake-agent`
does) through `SessionHost`, with an in-memory ledger, a fixed checkout, and a
plugin catalog that counts its leases. It covers record order and turn
attribution, command queueing, cancellation and discard on end, the crash window
and interrupted recovery, a mismatched plugin version, the Git identity, and
oversized records. The fixture exists for tests only; the Node image copies
`ora-node` alone.

The durable adapter is documented in [session ledger](session-ledger.md).

## Production service

Add an `agent` section beside `node`, `process`, `clone` and `control` in the service configuration:

```json
"agent": { "deno_path": "/usr/local/bin/deno", "ready_timeout_ms": 30000 }
```

With a workload user, add `"workload_directory": "/var/lib/ora/agent"`. It is required exactly
when `process.workload_uid` is set, must be an absolute UTF-8 path that already exists, owned by
the Node's identity and writable by no one else, and must not overlap the Node home, the process
host directory or the clone root. The Node never creates it; the sandbox entrypoint does.

The Deno path must be absolute and the ready timeout positive. With this section the handshake
advertises `AgentSession`; without it the service rejects new session work but still settles
unfinished sessions on startup. Deployment supplies Deno and the existing installed plugin package.
The service composes `AgentSessions` with the persistent journal, checkout resolver and the same
`PluginInstaller::catalog()` used by plugin execution. Plugin recovery runs before sessions can start.

Cloud-controlled starts use `ControlledStartAgentSession { binding, command }`, requiring both
RuntimeControl and AgentSession capability. The worker checks Controller ownership, exact execution
identity and runtime permission before recording input and rechecks before starting. Bare starts
are limited to unfenced local IPC. Repeated starts report state and do not launch another actor.
New session commands require a live binding in the original control scope; closing, expiring or
changing it cannot authorize new input. Completed sessions return `SessionCommandRejected` with
`session_ended`. Accepted replies leave the socket before `command_arrived` wakes the actor.

A connection reserves at most 256 event slots per execution, including queued socket writes.
It reads bounded pages after its own last reserved sequence, so a full execution does not prevent
another execution or control reply from progressing. Only a successful durable exact ACK frees a
slot. Reconnection drops these cursors and replays the smallest still-unacknowledged sequence from
disk with unchanged content and identity. Agent recording continues while the send window is full.

Before the control listener opens, every unfinished session is ended as `interrupted`; the Agent
is never resumed. Graceful shutdown cancels live conversations, waits for plugin cleanup, commits
terminal evidence and only then releases the Node database lease. Failed Executed settlement stops
a queued turn before prompt submission; missing terminal persistence or an aborted actor stops
service admission and leaves durable recovery responsibility.

The approved agent-runtime D2 exception still applies: plugin stdio/process groups provide normal
shutdown cleanup, but a descendant ignoring closed stdio after SIGKILL is not yet contained by
host/guardian. The crash test proves echo-plugin exit and interrupted replay, not arbitrary orphan
reclamation. Process I/O containment remains a separate dependency.

`tests/standalone/agent_sessions.rs` and its submodules run the production Node executable with a
real clone and echo plugin: command deduplication, history equality, shared installation leases,
runtime closure, graceful stop, SIGKILL recovery, window saturation and exact replay. Controller
Cloud work-item relay remains outside this change.
