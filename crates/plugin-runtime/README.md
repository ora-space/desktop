# ora-plugin-runtime

`ora-plugin-runtime` owns the lifecycle and stdio protocol for sandboxed Ora plugin
processes. It launches a configured JavaScript entrypoint with Deno, waits for the
plugin's immutable method registration, correlates concurrent JSON-RPC calls, drains
plugin logs from stderr, and shuts down the complete child process tree.
Explicit shutdown resolves only after the supervised process exits. Exit observation distinguishes
intentional shutdown from startup, protocol, and unexpected process failures.

The crate does not discover, install, select, or configure plugins. Callers supply a
plugin identifier, an entrypoint, a Deno executable, and the exact method to invoke.
Application-level code remains responsible for mapping Ora capabilities to those
plugin methods.

Stdout is reserved for framed protocol messages. Each frame contains a four-byte
big-endian length, a one-byte frame type, and a UTF-8 JSON payload. Protocol failures
invalidate the process because the host can no longer safely correlate responses.
