# Ora Plugin Configuration

`ora-plugin-config` owns the host rules for immutable plugin Setting Declarations and mutable,
plugin-global Stored Setting Values. Its public API compiles `assets/config.json`, resolves
effective values and Configuration Completeness, and persists revisioned `store.json` files under
`<data-dir>/plugins/data/<namespace>/<name>/`.

Store paths follow the same `<namespace>/<name>` segments as [`PluginId`](../domain/src/plugin_id.rs),
including dotted name segments such as `ora-space.tavily-search`; they are not restricted to hyphen-only
[`Slug`](../utils/src/slug.rs) spelling.

The installed `assets/config.json` dispatcher supports three mutually exclusive version-one
shapes, distinguished by top-level keys.
Without `transport` or `hook`, the file is a settings-only Setting Declaration. With `transport`,
it is an MCP Configuration: the same settings subset plus exactly one MCP Transport (`stdio`
with a package-contained command, or `http` with an HTTPS URL and credential-free headers). With
`hook`, it is a Hook Configuration: one immutable Hook Protocol descriptor (a closed, versioned
`HookProtocol` enum, starting with `rtk-rewrite-v1`), a package-relative executable, a normalized
bare command alias, and an embedded tool version, plus an optional future settings subset. A file
declaring both `transport` and `hook` fails closed (`MixedContribution`) so one package can never
carry two contribution shapes. `compile_configuration_file` performs that dispatch and returns
which shape it compiled, so kind-aware callers can refuse a transport-bearing file on a non-MCP
package or a hook-bearing file on a non-hook package. The settings subset of an MCP or Hook
Configuration feeds the same editor and `store.json` machinery as a settings-only file.
Phase 1 restricts MCP setting types to `string`, `number`, and `boolean`; `secret`, `file`, and
`directory` are rejected with a dedicated error. HTTP headers must bind
through Setting references — a literal header value would be a way to bake credentials into the
package. HTTP URLs must be HTTPS, must not carry userinfo, must not contain a query string, and
must not contain a fragment. Bound text — stdio argument and environment literals, plus
Setting-reference prefix/suffix on args, env, and headers — must not contain control characters
(including CR/LF).

The crate does not render UI, expose filesystem paths to frontend callers, or start plugins.
ACP `mcpServers` mapping and live Session refresh belong to `ora-backend` Session setup.

## Agent declaration compiler

`compile_agent_configuration` is a separate, side-effect-free entry point for a version-two
Agent declaration. It reuses the scalar Settings compiler, validates the model-access capability,
and fingerprints the exact input bytes. Its diagnostics do not echo rejected values.
The existing `compile_configuration_file` dispatcher and configuration store still accept only
their existing shapes; this compiler alone does not enable package installation, personal
configuration, or model execution. Consumers must bind the declaration to the exact release and
negotiate model access before connecting that route.

The byte-stable JSON input/output vectors in
[`tests/fixtures/agent-configuration-contract.json`](tests/fixtures/agent-configuration-contract.json)
are consumed by the Rust tests and can be reused by other language implementations. Each `source`
is a JSON string so checkout line-ending conversion cannot change its declaration fingerprint.
See the [declaration format](../../docs/agent-configuration-declarations.md) for the supported
shape, version and error boundaries.

Declaration parsing is strict and bounded. A missing declaration file, or a package root that
cannot be traversed as a directory (`NotADirectory`), is reported as undeclared rather than as a
load failure, so list summaries stay consistent across platforms when the package tree is
corrupt or half-removed. Stored values are independent of installed versions, writes replace the
complete explicit override set atomically, and optimistic revision plus declaration-fingerprint
checks prevent stale editors from overwriting newer state. Recovery preserves malformed files
under a collision-free local-time backup name; if replacement or restoration cannot complete, the
caller receives an explicit failure instead of a recovered snapshot.
