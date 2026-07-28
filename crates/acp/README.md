# ora-acp

`ora-acp` is Ora's provider-neutral ACP v1 transport over newline-delimited JSON-RPC stdio. It turns an asynchronous reader and writer into a typed request client plus independent streams of session updates and control messages.

## Responsibilities

- `AcpPeer` owns the reader task and exposes an `AcpClient` for serialized writes.
- `AcpClient` correlates concurrent requests by `RequestId`, decodes typed responses, sends notifications, and answers agent-originated requests.
- Session notifications and low-volume control traffic are separated so the backend can apply per-session flow control without blocking connection-wide parsing.
- Protocol, framing, I/O, and response-decoding failures are normalized as `AcpError`.

## Boundaries and failure semantics

- Frames are newline-delimited JSON with an 8 MiB maximum. An oversized or malformed frame is fatal to the connection.
- Unmatched responses, stdio loss, and invalid response envelopes fail pending operations instead of being silently ignored.
- Recognized permission requests are emitted as `PermissionRequest`. Unknown agent-originated methods receive a correlated method-not-found response without terminating the connection.
- The connection-to-router update channel is intentionally unbounded. Per-session bounds and overflow policy belong to the backend runtime, where one noisy session can be isolated from others.
- Writes share one mutex so concurrent JSON-RPC frames cannot interleave.

This crate does not spawn provider processes, supervise reconnects, route updates to Ora sessions, or enforce session lifecycle policy. Those responsibilities belong to `ora-backend`. See [ACP Agent Runtime](../../docs/agent-runtime.md).
