# ora-plugin-runtime

`ora-plugin-runtime` owns the lifecycle and stdio protocol for sandboxed Ora plugin
processes. It launches a configured JavaScript entrypoint with Deno, waits for the
plugin's immutable capability registration, correlates concurrent JSON-RPC calls,
carries notifications in both directions, drains plugin logs from stderr, and shuts
down the complete child process tree.

The crate does not discover, install, select, or configure plugins. Callers supply a
plugin identifier, an entrypoint, a Deno executable, the permission flags the plugin
was granted, an optional working directory, a `HostRequestHandler` that serves the
plugin's requests to the host, and the exact method to invoke. Application-level code
remains responsible for mapping Ora capabilities to those plugin methods and for
deciding which host methods exist.

## Protocol

Stdout is reserved for framed protocol messages. Each frame contains a four-byte
big-endian length, a one-byte frame type, and a UTF-8 JSON payload. Protocol failures
invalidate the process because the host can no longer safely correlate responses.

A plugin declares both traffic directions once, in its `ora/register` notification:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "ora/register",
  "params": {
    "methods": ["agent/start", "effect/waitForIdle", "effect/restart"],
    "emits": ["agent/acp"],
    "effectSurfaces": [
      {
        "workspaceRelativePath": ".agents/skills",
        "materializationFormat": "skill_directory.v1",
        "coordination": "wait_for_idle_and_restart",
      },
    ],
  },
}
```

- `methods` lists what the host may `invoke`. Invoking anything else fails locally
  without reaching the plugin.
- `emits` lists what the plugin may send on its own initiative. Notifications outside
  the whitelist invalidate the connection: a plugin whose behaviour exceeds its
  declaration cannot be trusted to correlate correctly.
- `effectSurfaces` optionally declares Workspace-relative filesystem surfaces consumed by the
  runtime. The runtime crate parses the locator, format, and coordination tokens strictly;
  capability-specific code validates their meaning and derives the trusted consumer identity from
  the launched plugin rather than from this payload.
- Plugin requests to the host (`ora/storage/*` and the like) need no declaration: the
  launch-time handler decides what it serves and answers method-not-found otherwise.

Both fields are fixed for the life of the process; a second registration is a protocol
error.

## Traffic shapes

| Direction     | Shape                         | Correlation        | Bounded by     |
| ------------- | ----------------------------- | ------------------ | -------------- |
| Host → plugin | `invoke` request/response     | runtime request id | `call_timeout` |
| Host → plugin | `notify` notification         | none               | nothing        |
| Plugin → host | whitelisted notification      | none               | nothing        |
| Plugin → host | request served by the handler | plugin request id  | the handler    |

`call_timeout` deliberately covers `invoke` only. Notifications carry payloads whose
own protocol owns correlation and cancellation — ACP traffic streams for minutes, which
a control-call timeout would sever. Plugin-originated notifications arrive on the
unbounded receiver returned by `launch`; connection-wide backpressure would let one
noisy stream stall unrelated traffic, so bounded queues belong to each consumer.

When an `invoke` times out, its request id moves from the active correlation table into a
bounded 256-entry tombstone queue. A late, otherwise valid response to that known request is
discarded; a genuinely unknown id remains a protocol failure. This keeps local cancellation from
invalidating a healthy plugin without allowing stale-id memory to grow without bound.

`shutdown` only requests termination. A lifecycle owner that must prevent generation overlap uses
`shutdown_and_wait`, which returns after the supervisor has observed graceful exit or killed and
reaped the complete process tree after the configured timeout.

Every launch failure after process creation follows the same reaping boundary, including missing
stdio, registration timeout, contract rejection, and agent control-call failure. Unexpected process
exit also closes the plugin-originated notification receiver even while public runtime handles still
exist, so the connection owner can fail the generation and apply its restart policy immediately.

## Host requests

A registered plugin may send JSON-RPC requests (messages with a numeric or string `id`).
Each one is dispatched to the `HostRequestHandler` passed to `launch` on its own task, so a
slow host method never delays the reader that also carries responses to the host's own
calls, and concurrent requests are answered under their own ids whatever order they finish
in. The handler returns a JSON result or a `HostRequestError` (`code`, `message`, optional
structured `data`); unknown methods must be answered with `HostRequestError::method_not_found`
(`-32601`). A request before registration, or one whose id is neither a number nor a string,
is a protocol failure. Launches without host methods pass `NoHostRequests`.

The handler is bound to one process at launch; that binding, not anything in the request
params, is how a handler knows which plugin is calling.
