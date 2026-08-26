# ora-plugin-config::mcp

Compiles one MCP Plugin's `assets/config.json` into an immutable MCP Configuration: the optional
Settings subset plus exactly one MCP Transport.

## Responsibilities

- Distinguish the Settings-only shape from the MCP shape by the presence of a top-level
  `transport` member, without taking a kind hint from the caller.
- Compile exclusive Stdio or HTTP transport so mixed shapes (HTTP with a command, stdio with a
  URL) are unrepresentable.
- Reuse the Settings-only declaration compiler for the settings subset so the existing editor and
  `store.json` machinery consume MCP Settings unchanged.
- Reject control characters (including CR/LF) in every bound-text position: stdio argument and
  environment literals, and Setting-reference prefix/suffix on args, env, and HTTP headers.
- Reject HTTP URLs that are not HTTPS, that carry userinfo, a query string, or a fragment, and
  reject HTTP header values that are not Setting references.

## Non-responsibilities

- Kind policy — only `mcp` packages may ship this shape, and an MCP package must — belongs to
  `ora-plugin-manager`.
- Filesystem containment of a stdio command inside the installed package belongs to
  `ora-plugin-manager`.
- `ResolvedMcp`, Agent materialization, and workspace MCP selection are later slices.

## Failure semantics

Unknown fields, unknown schema versions, unknown transport types, reserved Setting types
(`secret` / `file` / `directory`), and bound-text / URL policy violations fail compilation with a
stable field path. A missing `transport` member is not an MCP error: that file compiles as a
Settings-only declaration.
