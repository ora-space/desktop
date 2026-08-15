# HTTP handlers

This directory is the Web adapter's HTTP edge. Each handler extracts transport
input, calls `ora-backend` or a Web-only filesystem service, and serializes the
matching `ora-contracts` response.

## Responsibilities

- Own Axum extractors, path/query/body mapping, and HTTP status projection
  through `WebApiError`.
- Own the private NDJSON stream seam in [`ndjson_stream`](ndjson_stream.rs):
  `data` / `error` / `end` frames, deferred request-lifecycle completion, and
  observation of the process shutdown token.
- Keep route families thin. Session, workspace, and spec watch handlers create
  a source and hand it to `ndjson_stream`; they do not duplicate framing.

## Non-responsibilities

- Business rules, persistence, ACP session actors, and spec discovery live in
  `ora-backend`.
- Path containment and native watching live in `ora-fs`.
- Desktop stream cancellation lives in the Tauri adapter.

## NDJSON streams

`watchAppEvents`, workspace watch, spec watch, and in-flight ACP load/prompt
responses share one framing path. On Ctrl+C the process cancels `AppState`'s
shutdown token before Axum drains connections. Streams then complete so a live
browser tab cannot pin the process.

When shutdown wins, the stream inspects already-queued items without waiting
for future events:

- a buffered terminal error becomes an `error` frame and a failure completion
- buffered data is discarded so producers cannot keep the process alive
- an empty or disconnected queue becomes `end` and a success completion

See [Web Server Runtime](../../../../../docs/web-server-runtime.md).
