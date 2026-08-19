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

Each plugin process is launched deterministically: `deno run --no-prompt --cached-only`, with
`--frozen --lock <package>/deno.lock` when the package ships a lockfile, exactly the `--allow-*`
flags its manifest declared (`PluginPermissions`), the package root as working directory so Deno
discovers the package's own `deno.json`, and `DENO_DIR` pointed at the host-owned dependency cache.
Module resolution therefore never reaches the network at launch; dependencies must already be in
the cache or vendored inside the package.

Registration (`ora/register`) carries the method set, the notification methods the plugin may emit,
the SDK version, and named contract versions. A `pluginApi` contract the host does not support
fails the handshake. Notifications whose method was declared in `emits` are delivered to
`subscribe_notifications()` subscribers; any other plugin-originated method is a protocol failure.

Stdout is reserved for framed protocol messages. Each frame contains a four-byte
big-endian length, a one-byte frame type, and a UTF-8 JSON payload. Protocol failures
invalidate the process because the host can no longer safely correlate responses.
