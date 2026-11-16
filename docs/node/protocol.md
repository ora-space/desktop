# Controller–Node Protocol

English | [中文](protocol.zh.md)

`ora-node-protocol` (`crates/node-protocol`) defines the version 1 wire contract between a Controller
and a Node: typed messages and a validated asynchronous frame codec. Transports, sessions, Git and
durable execution belong to its consumers ([Control session](control-session.md), the Node and the
Controller). The crate depends on no domain, plugin, filesystem or persistence crate; its public API
is exported from `lib.rs`. The root decision is the
[Controller–Node protocol](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/0-controller-node-protocol.md)
ADR.

## Wire format

```text
4-byte big-endian length | 1-byte frame type | JSON envelope
```

The length covers the type byte and the JSON, not the prefix. `MAX_FRAME_LENGTH` is 16 MiB. Every
message uses `NODE_MESSAGE_FRAME_TYPE = 0x01`, and the JSON `message_type` selects the message.
Over WebSocket, one binary message carries the type byte and JSON without the length prefix.

Every envelope has `protocol_version`, `message_type` and `payload`, plus correlation fields that
depend on the message: commands carry `operation_id` and `execution_id` (and optionally
`request_id`), events add a `sequence`, and `EventAck` names `(operation_id, execution_id,
sequence)`. Names and tags are snake_case:

```json
{ "protocol_version": 1, "message_type": "hello", "payload": { "controller_id": "controller-1", "supported_versions": [1] } }
```

`Hello` advertises a non-empty, duplicate-free version list containing the envelope version;
`HelloAccepted` selects a version and advertises a non-empty, duplicate-free capability set. Matching
a response to an earlier Hello and enforcing handshake order belong to the session.

## Capabilities

| Capability | Controller → Node | Node → Controller |
|---|---|---|
| `worktree_execution` | `ensure_worktree`, `remove_worktree` | `worktree_ready`, `worktree_failed`, `worktree_removed`, `worktree_removal_failed` |
| `repository_clone` | `clone_repository` | `clone_result` |
| `runtime_control` | `bind_runtime` and the `controlled_*` envelopes | `runtime_control_state` |
| `plugin_install` | `install_plugins`, `remove_plugins` | `plugins_result` |
| `agent_session` | `start_agent_session`, `submit_user_turn`, `end_session` | `thread_event`, `agent_session_ended`, `session_command_accepted`, `session_command_rejected` |
| `shared_checkout_sessions` | Up to 8 unfinished Agent sessions per checkout, with restore fencing | Refused starts end as `agent_failed{checkout_busy}` or `agent_failed{checkout_session_limit}` without closing the connection |
| `model_proxy` | Model-bound session starts | `model_adapter` in `hello_accepted` (see below) |
| `revision_delivery` | `deliver_revision`, `upload_grant` | `revision_result`, `upload_grant_needed` |
| `revision_restore` | `start_agent_session` with `prior_revision` | Restore failure codes in `agent_session_ended` |

Always available: `hello`, `heartbeat`, `get_execution_status` / `execution_status` and `event_ack`.
The codec validates declarations only; a session must still check the selected Node's capability
before dispatch.

### Runtime control

With `runtime_control`, Cloud-authorized work arrives wrapped with a `RuntimeBinding`: the tenant,
Workspace, sandbox and runtime generation; the Node id, incarnation and instance; the Controller,
control epoch and version; the control session and acting user; the operation, execution and Node
operation ids; `input_closed`; and issue and expiry times. `ControlledClone`, `ControlledPlugins`,
`ControlledStartAgentSession` and `ControlledDeliverRevision` pair one binding with one command and
reject a permit for another execution, operation or target before admission. `RuntimeControlState`
acknowledges input closure with the list of unfinished execution ids (an empty list is an observed
fact). See [Runtime control](../controller/runtime-control.md).

### Message contracts

- **Clone.** `clone_repository` carries `node_id`, `repository_url`, `snapshot_ref`, `base_commit` and
  `workspace_ref`. The repository URL is `https://` without userinfo, query or fragment; the snapshot
  ref names a canonical Binding UUID and exactly the base commit; the Workspace ref
  (`refs/heads/workspaces/<workspaceId>/<branch>`) names a canonical Workspace UUID and a branch other
  than `HEAD`. The former `repository`/`branch` shape, SSH and unknown spec fields (destinations,
  credentials, Git options) are rejected. `clone_ready` carries the original spec, the originating
  incarnation, `RepositoryId`, a Node-scoped path and a full commit id; `clone_failed` a structured code
  (including `snapshot_unavailable` and `base_commit_mismatch`) and `no_directory` or
  `retained { repository_id, path }` residue. Raw diagnostics are never carried. See
  [Repository clone](repository-clone.md).
- **Plugins.** Inputs name each canonical `<namespace>/<identifier>` once, with one universal HTTP(S)
  download or distinct per-target downloads, each with a lowercase SHA-256. Item failures are reported
  per plugin; `plugins_failed` means the whole execution could not run.
- **Agent sessions.** A session names the agent plugin and version, the clone execution whose checkout
  it uses (never a Node path), a Git identity that cannot corrupt a signature line, and the first user
  turn (text, at most 64 KiB), optionally `model_binding_id` and `prior_revision`. A session for a
  plugin-owned model adapter also carries `agent_configuration`: the plugin id, version and target,
  package SHA-256, schema and adapter contract versions, declaration fingerprint, configuration
  revision, validated Settings as JSON text (an object of at most 256 KiB), protocol and the bound
  model's id, name, context window and output limit. It always comes with a `model_binding_id` and
  never with a credential. A Node advertises what it can run that way as `model_adapter` beside
  `model_proxy` in `hello_accepted` (contract version, target, protocols); the Controller dispatches
  such a session only to a Node whose `model_adapter` admits the frozen configuration. An older Node
  advertises none and is never sent one. It would not ignore the unknown field: the message fails to
  decode, which is a protocol violation, so the Node closes the whole control session (`1002` on
  WebSocket, end of stream on IPC) and discards its unaccepted queued work, not just that session
  start (see [Control session](control-session.md#closing)).
  `thread_event`
  carries one settled `ora-history` record as an opaque JSON object of at most 256 KiB (`truncated`
  marks a shortened one) and shares the execution's gap-free sequence with the terminal
  `agent_session_ended`, whose `detail` is a short snake_case code. Commands carry a `command_id`;
  their replies have no sequence and need no acknowledgement, and a lost reply is recovered by resending.
- **Revisions.** A delivery names the ended session, the clone, the base commit, a ref under
  `refs/ora/revisions/`, the Git Server `git_repository_url`, a `delivery_ref` under
  `refs/ora/deliveries/`, the `workspace_ref` and one history object key. `revision_delivered`
  requires a final commit different from the base; `revision_unchanged` allows the base or, with
  `prior_revision`, the prior final commit. A session's
  `prior_revision { revision_id, final_commit, base_commit, git_repository_url, delivery_ref }`
  names the Revision it resumes; a delivery's carries only `revision_id` and `final_commit`. A
  session also carries the Workspace directory (`workspace_directory`) that
  [`ora-git list`](ora-git.md) shows. A failed restore ends the session as `agent_failed` with
  `prior_revision_unavailable` or `prior_revision_base_unavailable`.
- **Grants.** `upload_grant` and `upload_grant_needed` are memory-only: no sequence, no
  acknowledgement, never persisted or logged, and `PresignedUrl` redacts itself in `Debug`.
- **Worktrees.** Both commands carry a `WorktreeExecutionSpec` (resource identities, an opaque
  `RepositoryRef`, the Main Workspace binding, base ref, expected branch and a
  `NodeManaged { directory_name }` path policy). The codec checks only that values are non-blank; the
  Node validates paths before use. See [Worktree execution](worktree-execution.md).

Result families use disjoint `kind` tags inside the untagged `ExecutionResult`; the Revision family is
boxed to keep `ExecutionState` small.

## Identities, status and acknowledgement

Identities serialize as opaque strings; the codec rejects empty or whitespace-only values and preserves
accepted ones exactly. `ControllerId` and `NodeId` are persistent peers, `NodeIncarnationId` one
running Node, `OperationId` a Controller-owned business operation, `ExecutionId` one attempt at it, and
`Sequence` an event's position within an execution. An unknown outcome never authorizes a new
execution identity; the codec neither generates identities nor checks sequence monotonicity.

`ExecutionStatus` is `Unknown`, `Accepted`, `Running` or `Completed` with the retained terminal result.
`Unknown` means insufficient evidence, never permission to repeat a side effect. The outer reporter is
the current runtime; a completed result keeps its original runtime identity, and only persistent NodeId
equality is required between them (a newer incarnation may report an older result). Status replies
carry no sequence and never acknowledge an event; `EventAck` names the exact event. Durable acceptance
precedes acknowledgement and unacknowledged events are replayed actively, independent of queries.

## Using the codec

| Caller | Send | Receive |
|---|---|---|
| Controller | `write_controller_message(ControllerToNodeMessage)` | `read_node_message` → `NodeToControllerMessage` |
| Node | `write_node_message(NodeToControllerMessage)` | `read_controller_message` → `ControllerToNodeMessage` |

Each read returns one validated message, or `None` at a clean EOF before a frame. Each write validates,
serializes, checks size, writes and flushes one message, rejecting before any byte is written; a
successful flush does not imply receipt or execution. Use these functions at the transport boundary,
because constructors and direct serde conversion do not run semantic validation. Give each stream
direction one owner and serialize whole writes; the codec keeps no partial-frame progress across
cancellation and cannot resynchronize after an error, so a connection owner discards the stream and
lets session recovery reconcile. `FrameError` distinguishes `Io`, `InvalidLength`,
`UnsupportedFrameType`, `EncodeJson`, `DecodeJson` and `InvalidMessage`. The codec emits no logs.

Tests enter through `crates/node-protocol/tests/protocol.rs`, with per-family fixtures that compare
independent wire JSON with typed messages in both directions; run `cargo test -p ora-node-protocol`.
