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
answer the peer's pings, and the Node heartbeat keeps traffic flowing.
Hello negotiates the existing version, Node identity/incarnation and clone capability. The session
accepts clone, status and exact acknowledgement messages; unsupported/conflicting messages close it.
The Node sends heartbeats independently of the blocking Git owner and actively replays bounded pages
of unacknowledged clone events. Status replies do not acknowledge events.

Admission uses a bounded queue and a revocable session guard. Only durable admission happens under
that guard; Git runs afterwards. Disconnect or session revocation discards unaccepted queued work,
but cannot cancel already accepted clones. Reads, writes and command admission replies use the finite
`frame_timeout_ms` deadline; an admission reply's deadline starts when its frame arrives.

Reading never waits for the worker. The session reads the next frame while earlier requests wait for
their answers, so a peer's end of stream, a WebSocket close or ping, and a truncated frame are seen
even while Git occupies the worker: a router that restarts mid-clone gets its close honored and the
control slot released at once, instead of the reconnect being refused with `4409`. Answers leave in
request order. At most 15 requests may be unanswered (one worker queue slot stays free for replay).
Beyond that, status queries are dropped, because the Controller polls on a timer and asks again, and
any other message closes the session so the Controller reconciles it by query after reconnecting. A busy worker can therefore cause a query/command session to close even
while heartbeats are arriving; reconnect queries the original execution, not a new attempt. An idle
Controller should periodically query its executions. Slow readers may be disconnected and reconnect
for replay. Shutdown closes admission and then performs the existing managed-process cleanup.

A real WebSocket test runs the production Controller session against a production Node, takes over a
clone result, observes the `4409` refusal of a second connection and rejects a mismatched Node identity. Another
keeps a clone result unacknowledged across a WebSocket disconnect and a Node restart, verifies the
identical replay, lets the production Controller session take it over and acknowledge it, and then
confirms the event is no longer replayed.

The real standalone test verifies owner/duplicate rejection, HTTPS clone, Node kill/restart, unchanged
result replay and exact acknowledgement. [Controller acceptance](../controller/local-runtime.md) adds
an independent-process durable-takeover and lost-Ack recovery test.

Additional real-socket tests pause HTTPS during an accepted clone, observe live heartbeats, expire a
queued command and verify it remains Unknown while the accepted clone completes. With Git paused in the
worker, further tests show that an IPC peer's end of stream and a WebSocket close release the control
slot at once, that WebSocket pings are answered, and that polling past the unanswered bound keeps the
session and is answered once Git finishes. Partial-frame tests
verify timeout and fresh admission; a non-reading peer is flooded with status replies until disconnect,
then reconnects to the identical unacknowledged result. No production test-only wire messages are used.
