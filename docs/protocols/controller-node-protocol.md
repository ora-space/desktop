# Controller–Node Protocol

English | [中文](controller-node-protocol.zh.md)

`ora-node-protocol` defines the version 1 wire contract for Controller–Node session messages and
Worktree execution. It provides typed messages and a validated asynchronous frame codec. Transport,
session orchestration, Git operations and durable execution belong to its consumers.

The contract supports a Worktree loop in which the target Node already has a Main Workspace.
Node-scoped resource references keep the same meaning across local IPC, SSH and network transports.
The crate has no dependency on `ora-domain`, `ora-plugin-protocol`, filesystem or persistence
implementations; its public API is exported from
[`lib.rs`](../../crates/node-protocol/src/lib.rs).

## Using the codec

Function names identify the **sender**, including for read operations:

| Caller     | Send                                                      | Receive                                                       |
| ---------- | --------------------------------------------------------- | ------------------------------------------------------------- |
| Controller | `write_controller_message` with `ControllerToNodeMessage` | `read_node_message` returning `NodeToControllerMessage`       |
| Node       | `write_node_message` with `NodeToControllerMessage`       | `read_controller_message` returning `ControllerToNodeMessage` |

The public functions accept `AsyncRead + Unpin` or `AsyncWrite + Unpin` streams. Message types are
fixed by direction; arbitrary serializable messages are not a public codec extension point.
Use these functions at the transport boundary: constructors and direct Serde conversion do not
perform the codec's semantic validation.

Each read returns one validated message, or `None` for EOF before a new frame. Each write validates,
serializes, checks frame size, writes and flushes one message. Validation, serialization and size
rejection occur before any message bytes are written. An I/O failure can leave partial bytes;
successful flush does not imply peer receipt, execution or durable acceptance.

Give each stream direction one owner and serialize complete writes. These operations do not retain
partial-frame progress across cancellation: do not cancel a read or write and then resume framing
on the same stream. The codec also provides no resynchronization after an error. A connection owner
should discard that stream and let session recovery reconcile outstanding work.

## Wire format and validation

```text
4-byte big-endian length | 1-byte frame type | JSON envelope
```

The length covers the type byte and JSON bytes, excluding the four-byte length prefix.
`MAX_FRAME_LENGTH` is 16 MiB, so the JSON payload is at most 16 MiB minus one byte.
All messages use `NODE_MESSAGE_FRAME_TYPE = 0x01`; the JSON `message_type` selects the business
message. This is binary framing around JSON, with no alternate `JsonDebug` encoding.

Every envelope contains `protocol_version`, `message_type` and `payload`. Correlation fields
depend on the message:

| Direction         | Messages                                                                      | Required correlation                       | Optional correlation |
| ----------------- | ----------------------------------------------------------------------------- | ------------------------------------------ | -------------------- |
| Controller → Node | `Hello`                                                                       | None                                       | None                 |
| Controller → Node | `EnsureWorktree`, `RemoveWorktree`                                            | `operation_id`, `execution_id`             | `request_id`         |
| Controller → Node | `GetExecutionStatus`                                                          | `operation_id`, `execution_id`             | None                 |
| Controller → Node | `EventAck`                                                                    | `operation_id`, `execution_id`, `sequence` | None                 |
| Node → Controller | `HelloAccepted`, `Heartbeat`                                                  | None                                       | None                 |
| Node → Controller | `ExecutionStatus`                                                             | `operation_id`, `execution_id`             | None                 |
| Node → Controller | `WorktreeReady`, `WorktreeFailed`, `WorktreeRemoved`, `WorktreeRemovalFailed` | `operation_id`, `execution_id`, `sequence` | `request_id`         |

Names and enum tags use snake_case on the wire. For example, the JSON inside a Hello frame is:

```json
{
  "protocol_version": 1,
  "message_type": "hello",
  "payload": {
    "controller_id": "controller-1",
    "supported_versions": [1]
  }
}
```

Both codec directions enforce envelope version 1. `Hello` advertises a nonempty, duplicate-free
version list containing the envelope version; it may also advertise other versions.
`HelloAccepted` selects the envelope version and advertises a duplicate-free capability set
containing `worktree_execution`, currently the only defined capability. These checks establish
message self-consistency. Matching the response to a previous Hello and enforcing handshake order
require a session implementation. A heartbeat carries the current Node identity, not execution evidence.

Required fields, known enum variants and direction are checked during decoding. Payload compatibility
means satisfying the selected message's structure: create and remove commands intentionally share
the same spec shape. Unknown object fields are generally ignored, so decoding is not a strict
rejection of every extra field.

`FrameError` distinguishes `Io`, `InvalidLength`, `UnsupportedFrameType`, `EncodeJson`,
`DecodeJson` and `InvalidMessage`. Missing fields, wrong-direction messages and incompatible
payload shapes are decoding errors; structurally valid messages that violate semantic constraints
carry a `MessageValidationError`. Truncated frames retain the I/O error kind, distinct from clean EOF.

## Identity and Worktree boundaries

Identity values serialize as opaque strings. The codec rejects empty or whitespace-only identities
and required domain strings, but preserves accepted values without trimming or normalization.

| Identity                    | Meaning                                                          |
| --------------------------- | ---------------------------------------------------------------- |
| `ControllerId`, `NodeId`    | Persistent peer identities                                       |
| `NodeIncarnationId`         | One running instance of a Node                                   |
| `RequestId`                 | Original logical Client command, when propagated                 |
| `OperationId`               | Controller-owned business operation                              |
| `ExecutionId`               | Execution attempt for that operation                             |
| `WorkspaceId`, `WorktreeId` | Workspace and managed task worktree                              |
| `Sequence`                  | Event position within an execution, serialized as a `u64` number |

`OperationId` and `ExecutionId` are distinct. The Worktree contract retains the original identities
and input on retransmission or restart; an unknown outcome does not authorize a new execution
identity. The codec carries these values without generating identities, tracking attempts or checking
sequence monotonicity. Zero is representable as a sequence; ordering is a consumer responsibility.

Both Worktree commands carry a `WorktreeExecutionSpec`: resource identities, an opaque
`RepositoryRef`, Main Workspace binding, base ref, expected branch and a
`NodeManaged { directory_name }` path policy. Node chooses and authorizes the worktree root.
The codec checks required values are nonblank; it does not resolve repositories, validate Git syntax,
normalize paths or enforce containment. Even a nonblank directory name still requires Node-side
path validation before filesystem use.

Creation success returns Node-scoped path, branch and base commit facts. Removal success distinguishes
`Removed` from idempotent `AlreadyAbsent`; failures carry a stable code and nonblank diagnostic
message. Consumers use the code for decisions and treat `NodePath` as a value in the target Node's
filesystem namespace, never as a local Controller or Client path.

## Status, historical results and acknowledgement

`GetExecutionStatus` addresses the original operation and execution on a Node.
`ExecutionStatus` represents `Unknown`, `Accepted`, `Running`, or `Completed` with a retained
terminal result. `Unknown` means insufficient evidence; it is not permission to repeat an external
side effect.

The outer `ExecutionStatus.node` identifies the current reporter. A Completed result preserves its
original runtime identity across all four terminal variants:

| Reporter               | Retained result        | Codec outcome                         |
| ---------------------- | ---------------------- | ------------------------------------- |
| Node A / incarnation 2 | Node A / incarnation 1 | Accepted, both identities preserved   |
| Node A / incarnation 2 | Node B / incarnation 1 | Rejected with `CompletedNodeMismatch` |

Only persistent NodeId equality is required. Session/Controller code must also verify the reporter
against the authenticated session and the execution's dispatched Node. Internal consistency alone
does not establish trust.

Status replies have no event sequence and do not acknowledge or deliver an original event.
`EventAck` identifies the exact `(execution_id, sequence)` being acknowledged.
The [recovery contract in D4](../../specs/decisions/node/protocol/0-controller-node-protocol.md#d4身份能力和会话恢复)
requires durable acceptance before acknowledgement and active replay of unacknowledged events,
independent of status queries. Consumers must handle either arrival order without duplicate business
effects; lost acknowledgements may be repeated, while queries of acknowledged executions do not
restart cleaned-up event delivery.

The crate provides the message representation for that contract. Session binding, reconnect,
deduplication, persist-before-side-effect, persist-before-ack and crash recovery require stateful
Node and Controller implementations and their own tests.

## Diagnostics and verification

Protocol logging is deferred under `todo-87602f0b`; the codec currently emits no logs.
English TODOs at the actual codec call sites specify future events and fields. Planned
`ora_logging` diagnostics use TRACE for typed messages and available frame metadata/error categories
for failures before decoding. See the [logging event map](controller-node-logging.md).

The [validation evidence](controller-node-validation.md) maps protocol guarantees to public-interface
tests and records unverified boundaries. Run `cargo test -p ora-node-protocol` and
`cargo clippy -p ora-node-protocol --all-targets -- -D warnings` for the crate's tests and lint.
