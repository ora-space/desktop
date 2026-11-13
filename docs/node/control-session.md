# Control Session

English | [中文](control-session.zh.md)

A Node accepts one Controller control session on exactly one listening endpoint. Both transports
carry the same frame body and run the same session code. The messages themselves are in
[Protocol](protocol.md). The decisions are the
[frame channel with IPC and WebSocket](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/20260924-frame-channel-with-ipc-and-websocket.md)
and [Controller heartbeat and WebSocket close codes](https://github.com/ora-space/specs/blob/main/decisions/node/protocol/20260925-controller-heartbeat-and-websocket-close-codes.md)
ADRs.

```json
{
  "control": {
    "controller_id": "deployment-controller",
    "listen": { "kind": "ipc", "path": "/home/node/state/control.sock" },
    "heartbeat_ms": 1000,
    "frame_timeout_ms": 10000
  }
}
```

| `listen.kind` | Use |
|---|---|
| `ipc` | Local deployments: a private Unix socket directly inside the Node home; each frame body is preceded by a four-byte big-endian length |
| `mutual_tls_websocket` | Sandboxes: `bind`, `path` and `tls` (certificate, private key, CA and an optional pinned peer certificate); one binary message per frame body |
| `websocket` | Plaintext, for loopback test fixtures only; the production executable refuses it |

A network listener also requires `process.workload_uid`, so workloads never share the management
identity. An optional `target` (tenant, Workspace, sandbox, runtime generation) is the
platform-assigned runtime scope: a valid Controller certificate cannot choose another tenant, and a
runtime binding outside it is refused. Over WebSocket, text messages, other paths and messages above
the 16 MiB frame limit are refused. In the cluster, Controllers reach Nodes through the sandbox router,
see [Sandbox routing](../operations/sandbox-routing.md). `control` requires the `clone` section;
without `control` the executable only recovers work it already accepted.

## Ownership

The deployment binds a persistent ControllerId, not the first peer to connect, and reconfiguring to
another owner fails. New clones are attributed to that Controller atomically with acceptance (schema
v4); older unclaimed records are neither replayed to nor acknowledged by the session.

The IPC path's private directory and the Node database lease protect endpoint recovery: only a
same-owner private socket that refuses connections may be replaced; files, symlinks and live
listeners are preserved.

One connection owns the handshake and control slot. Others are refused without replacing it: IPC closes
the socket, while WebSocket completes the upgrade and closes with `4409` (`control session busy`),
because routers forward close codes but turn HTTP refusals into gateway errors that look like an
unreachable Node. An IPC peer cannot tell "busy" from a ControllerId mismatch.

## Closing

Either side that ends an established WebSocket session on purpose sends a close code, so a router
forwards the reason instead of reporting a lost connection:

| Code | Meaning |
|---|---|
| `1001` | The process is stopping |
| `1002` | Protocol violation |
| `4403` | The peer is not the configured counterpart or lacks a needed capability |
| `4408` | The peer sent nothing or stopped reading within the I/O deadline |
| `1011` | The sender's own failure (persistence or admission) |

The receiving side answers the close so neither waits out a close-handshake timeout; the Node finishes
that handshake off the admission path, so a lingering Controller cannot make the next connection busy.
IPC carries no code and only ends the stream. Whatever its code, a close only means "connection
unavailable".

## Admission and the worker

Hello negotiates the protocol version, Node identity and incarnation, and capabilities. The session
accepts the commands of the negotiated capabilities, status queries and exact acknowledgements;
unsupported or conflicting messages close it, and Cloud mutations require their controlled envelope.

- Admission uses a bounded queue and a revocable session guard. Only durable admission happens under
  the guard. A disconnect or revocation discards unaccepted queued work but cannot cancel accepted
  clones. Reads, writes and admission replies use the `frame_timeout_ms` deadline, measured for a
  reply from when its frame arrived.
- The worker that answers admission owns the Node database and never waits on Git. Every clone step
  that runs Git runs on a separate clone executor one step at a time, with the worker recording
  each step's input before handing it over
  ([decision](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20260925-clone-git-effects-run-outside-the-admission-worker.md)).
  Status, acknowledgements and replay are therefore answered while Git waits on a remote. A stop lets
  the outstanding step finish for up to `shutdown_grace_ms + cleanup_timeout_ms`; anything left is
  settled from the recorded attempt on the next start.
- Reading never waits for the worker: an end of stream, a WebSocket close or ping, and a truncated
  frame are seen at once, even while Git runs, so a router that restarts mid-clone gets its close
  honored and the slot released. Answers leave in request order; at most 15 requests may be unanswered.
  Beyond that, status queries are dropped (the Controller polls again) and any other message closes
  the session.
- Heartbeats are independent of Git and plugin downloads. Bounded replay pages include unacknowledged
  events of every family; status replies never acknowledge.

## Liveness

The Node's per-frame read deadline is also the Controller liveness deadline. On every
`query_interval_ms` tick the Controller sends exactly one frame: a status query for a pending dispatch
or, when nothing is pending, a Controller `heartbeat` with its `controller_id`. The Node handles that
heartbeat in the read loop without entering the worker queue and ends the session if the id is not its
owner. An idle session therefore stays open, while a vanished Controller or half-open connection
releases the slot within `frame_timeout_ms`. Keep `query_interval_ms` at most half of the Node's
`frame_timeout_ms`; the two live in different processes and cannot be checked together. Neither side
sends WebSocket pings; both answer the peer's. Controller and Node must run the same release.
