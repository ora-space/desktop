# Plugin protocol and SDK bindings

`crates/plugin-protocol` owns the wire contract shared by Ora's Rust host, test plugin processes,
and `packages/plugin-sdk`. It contains the framed stdio codec, JSON-RPC method constants,
registration declarations, and DTOs for agent control, Effects, storage, child processes, and
workbench calls.

Run `task export-contracts` after changing a protocol DTO or method name. The Rust exporter writes
generated bindings into `packages/plugin-sdk/src/protocol`; `constants.ts` and the DTO modules are
generated, while `index.ts`, `json.ts`, and `transport.ts` remain hand-written. Do not duplicate a
wire name or shared payload in the SDK: add it to `ora-plugin-protocol`, regenerate, and consume the
generated export.

The plugin transport uses a four-byte big-endian frame length, followed by the one-byte JSON-RPC
frame type and its JSON payload. The length includes the frame type byte. Rust plugin processes
should call `ora_plugin_protocol::read_message` and `write_message`; TypeScript plugins use the
matching transport exported by the SDK.

Plugin SDK `0.8` uses snake_case for multi-word RPC path segments, including
`agent/list_models`, `effect/verify_ready`, and `ora/childprocess/close_stdin`. This is a wire
compatibility break: hosts and plugins must move to the new protocol together.

`agent/list_models` takes `AgentListModelsParams` — a single `cwd` naming the Workspace directory
the discovery is for. Adding the parameter is additive: a plugin that ignores params still answers,
it just cannot vary its catalog by project. The host calls the method on demand rather than during
the agent handshake, and gives it a longer budget than an ordinary control call because answering
may require starting a process.
