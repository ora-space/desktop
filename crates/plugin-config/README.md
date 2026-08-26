# Ora Plugin Configuration

`ora-plugin-config` owns the host rules for immutable plugin Setting Declarations and mutable,
plugin-global Stored Setting Values. Its public API compiles `assets/config.json`, resolves
effective values and Configuration Completeness, and persists revisioned `store.json` files under
`<data-dir>/plugins/data/<namespace>/<name>/`.

Store paths follow the same `<namespace>/<name>` segments as [`PluginId`](../../domain/src/plugin_id.rs),
including dotted name segments such as `ora-space.tavily-search`; they are not restricted to hyphen-only
[`Slug`](../../utils/src/slug.rs) spelling.

`assets/config.json` has two shapes, distinguished by the presence of a top-level `transport`
key. Without it, the file is a settings-only Setting Declaration. With it, the file is an MCP
Configuration: the same settings subset plus exactly one MCP Transport (`stdio` with a
package-contained command, or `http` with an HTTPS URL and credential-free headers).
`compile_configuration_file` performs that dispatch and returns which shape it compiled, so
kind-aware callers can refuse a transport-bearing file on a non-MCP package. The settings subset
of an MCP Configuration feeds the same editor and `store.json` machinery as a settings-only file.
Phase 1 restricts MCP setting types to `string`, `number`, and `boolean`; `secret`, `file`, and
`directory` are rejected with a dedicated error. HTTP headers must bind
through Setting references — a literal header value would be a way to bake credentials into the
package. HTTP URLs must be HTTPS, must not carry userinfo, must not contain a query string, and
must not contain a fragment. Bound text — stdio argument and environment literals, plus
Setting-reference prefix/suffix on args, env, and headers — must not contain control characters
(including CR/LF).

The crate does not render UI, expose filesystem paths to frontend callers, start plugins, or pass
configuration to Agent processes. Callers supply package identity and roots; lifecycle and backend
layers map the resulting value-oriented types into their own contracts.

Declaration parsing is strict and bounded. A missing declaration file, or a package root that
cannot be traversed as a directory (`NotADirectory`), is reported as undeclared rather than as a
load failure, so list summaries stay consistent across platforms when the package tree is
corrupt or half-removed. Stored values are independent of installed versions, writes replace the
complete explicit override set atomically, and optimistic revision plus declaration-fingerprint
checks prevent stale editors from overwriting newer state. Recovery preserves malformed files
under a collision-free local-time backup name; if replacement or restoration cannot complete, the
caller receives an explicit failure instead of a recovered snapshot.
