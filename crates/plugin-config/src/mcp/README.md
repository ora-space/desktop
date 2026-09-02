# ora-plugin-config::mcp

Compiles one `assets/config.json` into the closed union of Settings-only, MCP, and Hook shapes.

The presence of a top-level `transport` member selects the MCP shape, the presence of `hook`
selects the Hook shape, and the absence of both falls back to Settings-only. The three shapes are
mutually exclusive: a file declaring `transport` and `hook` together fails closed
(`MixedContribution`) so one package can never carry two contribution types. Kind policy — an MCP
package must ship the MCP shape, a Hook package must ship the Hook shape, and every other kind
must not — belongs to `ora-plugin-manager`.

## Responsibilities

- Distinguish Settings-only, MCP, and Hook shapes without taking a kind hint from the caller.
- Compile exclusive Stdio or HTTP transport so mixed shapes (HTTP with a command, stdio with a
  URL) are unrepresentable.
- Dispatch a Hook-shaped file to the Hook compiler so protocol, executable, command alias, and
  tool version are validated with the same strictness as MCP transport.
- Reuse the Settings-only declaration compiler for the settings subset so the existing editor and
  `store.json` machinery consume MCP and Hook Settings unchanged.
- Reject control characters (including CR/LF) in every bound-text position: stdio argument and
  environment literals, and Setting-reference prefix/suffix on args, env, and HTTP headers.
- Reject HTTP URLs that are not HTTPS, that carry userinfo, a query string, or a fragment, and
  reject HTTP header values that are not Setting references.

## Non-responsibilities

- Kind policy — only `mcp` packages may ship the MCP shape, only `hook` packages may ship the
  Hook shape, and those packages must — belongs to `ora-plugin-manager`.
- Filesystem containment of a stdio command or Hook executable inside the installed package
  belongs to `ora-plugin-manager`.
- Binding Setting values onto a compiled transport (`resolve_mcp_transport`) stays in this crate;
  ACP `mcpServers` mapping, Session cwd substitution, and live refresh belong to Session setup.
- Hook runtime execution and workspace selection are later slices.

## Failure semantics

Unknown fields, unknown schema versions, unknown transport types, reserved Setting types
(`secret` / `file` / `directory`), mixed `transport`+`hook`, and bound-text / URL policy
violations fail compilation with a stable field path. A missing `transport` and `hook` member is
not an MCP or Hook error: that file compiles as a Settings-only declaration.
