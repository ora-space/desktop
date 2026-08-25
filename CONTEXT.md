# Ora Plugins

Ora's plugin marketplace, installation, and per-kind eligibility. This glossary names the kinds of plugin and the facts that exist after install, before any Agent uses them.

## Language

**MCP Plugin**:
A configuration-only plugin that describes one MCP Server. It has no Deno process and no `main.js`. Ora installs and parses the package; an Agent Plugin is what later turns that description into a target Agent's config.
_Avoid_: MCPB, MCP Registry `server.json`, MCP server plugin, process plugin

**Plugin Identifier**:
One or two `[a-z0-9-]` slug segments in the listing's `identifier` field. The registry id is `namespace/identifier`. Resolver v1 accepts only `namespace = "official"`. This Tavily plugin is `official/ora-space.tavily`.
_Avoid_: `name` in marketplace `orax.toml` (the implemented field is `identifier`)

**Plugin Kind**:
The closed set of plugin kinds resolver v1 can install: workbench, agent, webview, skill, and (in this work) mcp. Hook is not a kind until a later change adds it.
_Avoid_: hook (not a kind yet), MCPB kinds

**Agent Plugin**:
A process plugin that adapts one target Agent CLI. It is the only party that materializes MCP configuration into that Agent. An MCP Plugin author does not implement this.
_Avoid_: agent adapter, MCP host

**Installed**:
The plugin package is on disk and statically valid. Installed does not mean the user has finished settings, a remote endpoint is reachable, or any Agent has loaded the plugin.
_Avoid_: ready, connected, loaded, working

**Plugin Enabled**:
A user-controlled, software-global eligibility flag on an installed plugin. It is the toggle in Ora's settings UI. For an MCP Plugin it does not by itself select that MCP for any Workspace.
_Avoid_: activated, running, Workspace MCP selection

**Workspace MCP Selection**:
Which installed MCP Plugins a Workspace wants. Distinct from Plugin Enabled. Shared Sessions under the same Workspace cannot have conflicting MCP sets.
_Avoid_: session MCP, global enable

**MCP Transport**:
Exactly one connection description on an MCP Plugin: Stdio or HTTP. HTTP means MCP Streamable HTTP, not deprecated HTTP+SSE, and not a local process Ora starts. Host MCP Configuration compiles both shapes; a given package still has exactly one. This Tavily package is HTTP.
_Avoid_: SSE, mixed transports, npx, local HTTP server bundled in the plugin

**Secret Setting**:
A Setting whose value is a secret reference. Plaintext must not appear in the package, `store.json`, logs, or plugin protocol.
_Avoid_: string setting used for tokens, API key in URL, API key baked into the package

**MCP Configuration**:
The host compilation of one MCP Plugin's `assets/config.json` (settings plus the exclusive MCP Transport) into an immutable installed descriptor. It is not Agent Plugin materialization and not the user's `store.json`.
_Avoid_: MCPB manifest, server.json, ResolvedMcp

**Installed MCP Descriptor**:
The immutable result of MCP Configuration at install time. It proves the package is statically valid. It is not a ResolvedMcp and does not mean settings are filled.
_Avoid_: ResolvedMcp, ready, connected
