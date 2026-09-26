# Node control session

English | [中文](local-ipc.zh.md)

Linux `ora-node` optionally accepts one Controller control session on exactly one listening entry:
a private Unix socket for local deployments, or WebSocket for sandboxes reached through a platform
router. Both carry the same frame body (`[type][JSON]`) and run the same session code. Add a
`control` object alongside the [clone deployment configuration](repository-clone.md):

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

In a sandbox, listen for WebSocket upgrades instead:

```json
"listen": { "kind": "websocket", "bind": "0.0.0.0:9001", "path": "/ora-node/v1" }
```

Over IPC each frame body is preceded by a four-byte big-endian length. Over WebSocket one binary
message carries exactly one frame body; text messages, other paths and messages above the 16 MiB
frame limit are refused. The Node does not authenticate WebSocket peers: the listener must only be
reachable through the platform router, which authenticates the Controller, and platform credentials
must not be placed in the Node configuration or image. WebSocket ping/pong only proves the nearest
hop is alive; end-to-end liveness remains the protocol heartbeat and `frame_timeout_ms`.

The IPC path must be directly inside the injected Node home. That private directory and the Node
database lease protect endpoint recovery: only a same-owner private socket with a refused connection
may be replaced. Files, symlinks and live listeners are preserved. The optional section requires clone
configuration; without it the executable remains a recovery-only owner.

Deployment binds the persistent ControllerId, not the first peer. Reconfiguration to another owner
fails. Schema v4 attributes new clones atomically with acceptance; old unclaimed records stay intact
and are not replayed to or acknowledged by this session. This is trusted local ownership checking,
not cryptographic authentication or a sandbox against same-UID code.

One connection owns the handshake/control slot, whichever transport it arrived on. Other connections
are refused without replacing it: IPC closes the socket, WebSocket completes the upgrade and closes with
code `4409` (`control session busy`), because routers forward close codes but turn HTTP refusals into
gateway errors indistinguishable from an unreachable Node. IPC has no close code, so a refused IPC
peer only sees the socket close during the handshake; the Controller cannot tell "occupied" from a
Node that closed because the ControllerId does not match, and logs both as a protocol failure before
reconnecting after `reconnect_ms`. WebSocket keepalive pings are not sent by either side; both only
answer the peer's pings, and the heartbeats in both directions keep traffic flowing.

Either side that ends an established WebSocket session on purpose sends a close code, so a router
forwards the reason instead of reporting a lost connection (`1011`): `1001` when the process stops,
`1002` for a protocol violation, `4403` when the peer is not the configured counterpart or lacks a
needed capability, `4408` when the peer sent nothing or stopped reading within the I/O deadline, and
`1011` for the sender's own failure (persistence or admission). The receiving side answers the close
so neither waits out a close-handshake timeout. The Node finishes that handshake off the admission
path, so a Controller that lingers after a close cannot make the next connection busy; only a normal
stop waits for it, bounded by `frame_timeout_ms`. IPC carries no code and only ends the stream.
Every close, whatever its code, still only means "connection unavailable".

Hello negotiates the existing version, Node identity/incarnation and clone capability. The session
accepts clone, status and exact acknowledgement messages; unsupported/conflicting messages close it.
The Node sends heartbeats independently of the blocking Git owner and actively replays bounded pages
of unacknowledged clone events. Status replies do not acknowledge events.

Admission uses a bounded queue and a revocable session guard. Only durable admission happens under
that guard; Git runs afterwards. Disconnect or session revocation discards unaccepted queued work,
but cannot cancel already accepted clones. Reads, writes and command admission replies use the finite
`frame_timeout_ms` deadline. A busy worker can therefore cause a query/command session to close even
while heartbeats are arriving; reconnect queries the original execution, not a new attempt.

The Node's per-frame read deadline is also the Controller liveness deadline. On every
`query_interval_ms` tick the Controller sends exactly one frame: a status query for a pending
dispatch, or, when nothing is pending, a Controller `heartbeat` carrying its `controller_id`. The Node
handles that heartbeat inside the session read loop without entering the worker queue, so it never
waits behind Git, and ends the session if the `controller_id` is not its owner. An idle session
therefore stays open, while a vanished Controller or half-open connection releases the control slot
within `frame_timeout_ms`. Keep `query_interval_ms` well below the Node's `frame_timeout_ms` (at most
half of it); the two live in different processes' configuration and cannot be checked at startup.
Controller and Node must run the same release: an older Node rejects the Controller heartbeat.
Slow readers may be disconnected and reconnect for replay. Shutdown closes admission and then performs the existing managed-process cleanup.

A real WebSocket test runs the production Controller session against a production Node, takes over a
clone result, observes the `4409` refusal of a second connection and rejects a mismatched Node identity. Another
keeps a clone result unacknowledged across a WebSocket disconnect and a Node restart, verifies the
identical replay, lets the production Controller session take it over and acknowledge it, and then
confirms the event is no longer replayed.

The real standalone test verifies owner/duplicate rejection, HTTPS clone, Node kill/restart, unchanged
result replay and exact acknowledgement. [Controller acceptance](../controller/local-runtime.md) adds
an independent-process durable-takeover and lost-Ack recovery test.

Additional real-socket tests pause HTTPS during an accepted clone, observe live heartbeats, expire a
queued command and verify it remains Unknown while the accepted clone completes. Partial-frame tests
verify timeout and fresh admission; a non-reading peer is flooded with status replies until disconnect,
then reconnects to the identical unacknowledged result. No production test-only wire messages are used.
