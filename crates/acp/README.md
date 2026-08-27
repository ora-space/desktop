# ora-acp

`ora-acp` is Ora's provider-neutral ACP v1 peer. It turns one connection into a typed request client plus one ordered inbound stream for each session's updates, permission requests, and terminating responses.

The peer is transport-neutral: it consumes and produces whole JSON-RPC messages and never inspects framing. `AcpTransport` is the seam. Every agent Ora reaches is supplied by a plugin, whose IPC hands over already-parsed messages, so nothing here serializes a value that was never bytes.

ACP wire values come from the official `agent-client-protocol-schema` crate. `ora-acp` owns transport behavior and Ora-specific routing metadata, not a fork of the protocol schema.

## Responsibilities

- `AcpTransport` owns framing and write ordering: `send` delivers whole messages in call order, and the `AcpMessages` stream handed to `AcpPeer::spawn` yields exactly one message per frame.
- `AcpPeer` owns the routing task and exposes an `AcpClient` for writes.
- `AcpClient` correlates direct requests by `RequestId`, decodes typed responses, sends notifications, and answers agent-originated requests.
- Session trace registrations translate provider session identifiers into the Ora-owned identifiers used by the `session_id` log field.
- Session requests return a pending handle whose response is emitted into the same inbound stream as that session's updates and permission requests. The reader preserves their wire order, so a terminating response cannot overtake an earlier update.
- Cancelled direct requests and abandoned session requests retire their ids in a bounded tombstone registry. A late response for an abandoned request is discarded, while a genuinely unknown response remains a connection failure.
- Protocol, framing, I/O, and response-decoding failures are normalized as `AcpError`.

## Boundaries and failure semantics

- Framing limits belong to the transport. A malformed frame ends the inbound stream with that failure, which is fatal to the connection.
- Unmatched responses, a closed inbound stream, and invalid response envelopes fail pending operations instead of being silently ignored. The exception is a response for a cancelled direct request or deliberately abandoned session request, which is recognized by its bounded tombstone.
- Recognized permission requests are emitted as `PermissionRequest`. Unknown agent-originated methods receive a correlated method-not-found response without terminating the connection.
- The connection-to-router inbound channel is intentionally unbounded and preserves the reader's order. Per-session bounds and overflow policy belong to the backend runtime, where one noisy session can be isolated from others.
- Write serialization is the transport's guarantee: `send` must deliver whole messages in call order so concurrent JSON-RPC frames cannot interleave.

This crate does not spawn provider processes, supervise reconnects, route updates to Ora sessions, or enforce session lifecycle policy. Those responsibilities belong to `ora-backend`. See [ACP Agent Runtime](../../docs/agent-runtime.md).
