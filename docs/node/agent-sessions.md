# Node Agent Sessions

English | [中文](agent-sessions.zh.md)

A Node runs Agent session executions with the same [agent runtime](../agents/agent-runtime.md) Desktop
uses. In the checkout a clone execution left, it starts only the agent plugin the execution names, at
its exact version, writes every settled record to the session JSONL before handing it to the Node
ledger as a Thread event, and runs session commands in acceptance order. Code lives in
`apps/ora-node/src/session` (Linux only). The decisions are the
[shared agent runtime hosted by the Node](https://github.com/ora-space/specs/blob/main/decisions/node/agent-runtime/0-shared-agent-runtime-crate-hosted-by-node.md),
[streamed Thread events and session commands](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/20260928-streamed-thread-events-and-session-commands.md)
and the [run-scoped model proxy](https://github.com/ora-space/specs/blob/main/decisions/node/agent-runtime/20261010-run-scoped-model-proxy.md).

## Deployment

```json
"agent": { "deno_path": "/usr/local/bin/deno", "ready_timeout_ms": 30000 }
```

The Deno path must be absolute and the timeout positive. With this section the handshake advertises
`agent_session`; without it new sessions are refused but unfinished ones are still settled at startup.
When `process.workload_uid` is set, `agent.workload_directory` is required: an existing absolute path
owned by the Node identity, writable by no one else and not overlapping the Node home, the clone
root or a configured legacy `process.host_directory`. The sandbox entrypoint creates it; the Node
never does.
With `agent.ora_git_directory`, `git_access` and a `process.workload_uid` different from the Node's
effective UID, each session also gets its own
[ora-git](ora-git.md) socket, exported as `ORA_GIT_SOCKET`.

## One runtime per execution

Each session execution composes its own plugin lifecycle and agent runtime. Plugins are installed
under `plugins/` in the Node home (see [Node plugins](plugins.md)) and histories live under
`sessions/` beside it, but this lifecycle reports and starts only the execution's agent plugin, so
"only the named plugin" and "the identity reaches only this execution's processes" hold by
construction.

| Runtime interface | Node implementation |
|---|---|
| `SessionStore` | `MemorySessionStore`: a Node never resumes a session after restarting |
| `AgentAttach` | The execution's own one-plugin lifecycle; no Effect consumer (Nodes project no Skills) |
| `SessionSetup` | `NoSessionMcp`: MCP configuration has no delivery path on a Node |
| `RuntimeEvents` | Every history line becomes a Thread event; title and model-catalog events are dropped |
| `WorkspaceDirectory` | The checkout |

A host with no person present uses runtime entry points in place of user actions: `wait_for_agent`
waits, unbounded unless the caller bounds it, until the agent connection is `Ready` or its supervisor
gives up; `start_session_with_id` names the session after the durable execution identity (an identity
already in use is refused); `prompt_session_as_message` records the host's message identity as the ACP
`messageId` of every block of the user message.

The run's Git identity is exported as `GIT_AUTHOR_*`/`GIT_COMMITTER_*` on the plugin process and on
every process the host spawns for it; the Node writes no Git configuration. For a session with a
frozen Agent configuration, these variables and the session's other own variables join an otherwise
empty environment that keeps only `PATH`, `LANG`, `LC_ALL` and `TMPDIR`, so model credentials, proxy settings and CLI configuration in
the Node's environment never reach the agent (see
[Agent configuration](../plugins/agent-configuration.md)). Any other session layers them on the
inherited environment, where the Node's own model credentials and proxy settings still reach the
agent.

### Separate workload user

With `process.workload_uid`, the Deno plugin and everything it asks the host to spawn drop to the
workload UID (group equal to it, no supplementary groups, no capabilities, `no_new_privs`, umask
`077`); a spawn that cannot drop fails. Each session gets `<workload_directory>/<sha256(execution_id)>/`
(root-owned, `0711`) with:

- `package/`: a hard-link view of the installed package in fresh `0755` directories (copied with
  `0644`/`0755` across filesystems; links and special files refuse the view). The plugin is launched
  from the view while the lifecycle still verifies the installed package.
- `home/`: `0700`, owned by the workload user. `HOME`, the XDG directories, `DENO_DIR` and
  `DENO_NO_UPDATE_CHECK=1` point there for the plugin and host-spawned processes alike.

Before launch the checkout is handed to the workload user without following links. The session
directory is removed once the plugin stopped, and leftovers are removed when the service opens.

## How an execution runs

1. `start` returns at once; the session resolves the checkout in the background and ends as
   `agent_failed{checkout_unavailable}` without one.
2. It takes the plugin's use lease before looking up the version. A missing version, or a package that
   is not that version or not an agent, ends as `agent_failed{agent_plugin_unavailable}` with no process
   started. The lease is held until the plugin's whole process tree exits.
3. With a `prior_revision`, the session restores it into the checkout before the checkout is handed to
   the workload user and before any plugin starts, see [Revisions](revisions.md#restoring-a-prior-revision).
4. It waits, bounded, for the agent connection (`agent_failed{agent_unavailable}` on timeout), creates
   the session with the execution id as its Ora Session id (`agent_failed{agent_start_failed}` on
   failure), and sends `initial_turn`.
5. Every history line settled during a turn becomes a Thread event with that turn's `turn_id`; the user
   message carries the same identity as its ACP `messageId`. A record over 256 KiB keeps only `at`,
   `seq` and `type` in the Thread and is marked `truncated`, while the JSONL keeps the original.
6. `command_arrived` wakes the session to read its whole queue. A `SubmitUserTurn` runs after the
   current turn, in acceptance order, and is settled `executed` when it starts. An `EndSession` settles
   the turns accepted before it as `discarded`, stops the session (cancelling a running turn and
   recording `TurnEnded{cancelled}`), then settles itself `executed`.
7. At the end the session stops, releases its runtime, stops the plugin and waits for its tree to exit,
   releases the lease, discards any queued command, writes the terminal result and forgets the session.

Only a successful ACP completion admits another turn. A prompt error or unexpected stream closure ends
the session as `agent_failed` with `agent_turn_failed` or `agent_stream_closed`; no raw provider
diagnostic enters history or logs, and an `EndSession` already queued keeps its explicit reason.

## Scheduled run business facts

Cloud can admit a session spec as a scheduled occurrence (`spec.trigger = scheduled`) instead of a
manual Issue run, where the field is absent. Only such a session, and only one Cloud ended as asked
(`user_ended` or `idle_timeout`) after the agent finished a turn, reports business facts in its
terminal result; a cancelled, failed or interrupted session, every manual run, and a run whose final
turn was cancelled or never ended all report neither fact. The end reason alone is not proof that a
turn completed: an end command cancels the turn that is still running. A completed turn that only
used tools states no verdict, and its artifacts are still declared.

- The verdict is parsed from the final Assistant turn of the sealed history, and only when
  `TurnEnded` closed that turn with `end_turn`:
  `QA_VERDICT: <pass|warn|fail|blocked|idle|range_unresolved>`, optionally followed on the next line
  by `QA_DETAIL: ...`. The first marker wins, matching is case-insensitive, and an illegal value, a
  detail over 2000 characters or no marker at all simply reports no verdict. A detail the wire
  refuses — one containing a NUL, for instance — is treated the same way, so a malformed annotation
  never fails the transaction that settles the run. An earlier turn is never used instead of the
  final one. Node never parses a Thread on Cloud's behalf; it reports the typed value
  (`scheduled_result`) and Cloud verifies it.
- Artifacts are read from the fixed `.ora-scheduled-artifacts/` directory of the checkout before the
  Workspace can be released. Collection is read-only and applies the closed type whitelist
  (markdown, json, txt, log, xml, html), the same path, symbolic-link and hard-link refusals as local
  scheduled runs, and the per-file 10 MiB, per-run 64-artifact and 64 MiB total bounds. A name is
  identified after NFC normalization and Unicode case folding, so two spellings one case-insensitive
  filesystem treats as one path are declared once and the second is refused. Accepted files become
  the declared `artifact_manifest` with their size and SHA-256, which Cloud verifies against the
  upload; refused candidates stay in the manifest with their reason instead of disappearing. The
  walk spends a 512-entry budget while it reads each directory, so a flooded directory is never
  enumerated, kept or sorted in full. It lists at most 64 refusals and counts further ones in
  `omitted_rejections`; when it stops before reading every entry it sets `walk_truncated`, so
  candidates nobody examined are never mistaken for a complete list of refusals. A missing directory
  declares nothing, and Cloud chooses every object key later; Node never writes to the Workspace and
  never holds an object-store credential. Uploading the
  declared files is the separate `deliver_scheduled_artifacts` execution described in
  [Revisions](revisions.md#scheduled-artifact-uploads).

## Live previews

Live previews require both peers to opt into the optional Hello/HelloAccepted `thread_previews`
field. Runtime emits pending prose, thought and tool replacements at most every 100 ms. Node retains
only the latest version per execution: at most 32 blocks and 256 KiB per snapshot, up to 64 sessions.
Blocks keep their assigned history position; settled `ThreadEvent.history_sequence` supersedes
that position. Previews never enter SQLite, carry no event sequence or Ack, and disconnects or slow
consumers never end the Agent. Reconnect reads a full current snapshot. Peers without negotiation
retain durable-only delivery. Store capability selection is described in
[Controller](../controller/controller.md#coordination-store).

## Record order and crashes

A settled assistant message carries an optional plain-text projection in the Node protocol for a
later task handoff. Node derives it from typed `AgentMessageChunk` content; thoughts, tools and
non-text blocks produce none. Each projection is bounded to 64 KiB and clipped at a UTF-8 boundary
with an explicit truncation flag. The original JSONL and Thread record remain unchanged. Consumers
must treat an absent projection as unavailable history instead of guessing from tool or thought
records.

The runtime calls `record_settled` synchronously after a line is in the file and before the next is
written, and the Node appends the Thread event there, so every Thread record is in the JSONL in the same
order and a crash leaves the JSONL at most one line ahead. Once a Thread append fails, the mirror stops
for good and the session ends as `agent_failed{thread_unavailable}`.

After a restart, agents are never resumed. An unfinished restore first hands over its original
Git attempts; only stop confirmation permits `interrupted`. Unknown original stop evidence keeps the
session nonterminal and fences delivery and checkout reuse (see [Revision recovery](revisions.md)). Graceful shutdown cancels live conversations, waits for plugin
cleanup and commits terminal evidence before releasing the database lease. `sealed_history` answers
`history_unavailable` while a session still runs and otherwise returns its JSONL.

The plugin process and agent CLI are started by the Node directly (process group, whole-tree
termination) and are not registered with the Node's `ora-reaper`. When the Node crashes, the plugin
exits on its closed stdio, but a descendant that ignores that is not reclaimed.

## Ledger

The session ledger lives in `ora-node-db` (schema v8); `apps/ora-node/src/session/ledger.rs` implements
the `SessionLedger` and `CheckoutResolver` ports over `SessionJournal`.

- `NodeDatabase::session_journal` opens a narrow connection sharing the Node's exclusive lease; writes
  use immediate transactions with `FULL` durability, and the last journal keeps the lease alive.
- Admission keeps the complete immutable input. Controlled admission records the runtime permit and
  respects unfinished responsibilities; start rechecks authority and only moves `Accepted` to `Running`.
- Thread appends require `Running` and allocate the next sequence in the same transaction. Exact ACKs
  delete only the named event; sequencing never resets.
- Commands keep their envelope, unique id and acceptance order. Identical retries never requeue a
  settled command and changed input fails; settlement is one-way.
- Ending a session commits its result, final event and the disposal of every queued command together.
  Later commands get `SessionCommandRejected(session_ended)`.
- Checkout lookup returns the stored target path only for a completed successful clone and fails
  closed otherwise; nothing is reconstructed from ids.

Cloud-controlled starts use `ControlledStartAgentSession`, requiring `runtime_control` and
`agent_session`; bare starts are limited to unfenced local IPC. A connection reserves at most 256 event
slots per execution and pages after its own last reserved sequence, so a full execution never blocks
another or the control replies; only a durable exact ACK frees a slot, and reconnecting replays the
smallest unacknowledged sequence. Recording continues while the send window is full.

## Model access

`AgentSessionSpec.model_binding_id` is an optional opaque Cloud reference. A deployment adds
`agent.model_proxy` with absolute `gateway_url`, `ca_cert`, `client_cert` and `client_key`; that
client certificate is issued for the runtime's tenant, Workspace and generation, separate from the Node
WSS server certificate. The service validates it before advertising `model_proxy`.

- The Node requests `POST /internal/v1/model-grants` with only the binding and execution ids. The
  platform returns a frozen protocol and model and a temporary in-memory token, never an upstream key.
- OpenCode uses one `ora-model` provider with the OpenAI-compatible or Anthropic SDK, reads the token
  from `ORA_MODEL_ACCESS_TOKEN` through an environment placeholder, and trusts the platform CA through
  `NODE_EXTRA_CA_CERTS`. With a workload user, HOME, XDG directories and `OPENCODE_CONFIG_DIR` use the
  session home and only the public CA is published there; otherwise a session-private directory under
  `model-runtime/` is used. Both stay outside the checkout and the Revision, and the token is never
  written to disk.
- Model-bound OpenCode sets `OPENCODE_DISABLE_PROJECT_CONFIG=true`, so checkout `.opencode`
  configuration is ignored and nothing lands in a Revision.
- Renewal extends the grant without changing the token; a denial cancels the conversation. Normal
  termination stops renewal, revokes the grant and removes temporary state before the result is
  recorded. A verified `model_session_ending` denial waits at most 30 seconds for the durable
  `EndSession`. A crashed Node never resumes: Cloud recovery and grant expiry revoke its authority.
