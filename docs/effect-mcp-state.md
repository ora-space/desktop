# MCP Effect materialization

Ora publishes every installed, complete MCP plugin as a secret-free `ora/mcp` Effect revision.
The normal Workspace Effect worker projects those revisions into Agent-declared shared files:
`.opencode/opencode.json` for OpenCode and `.mcp.json` for Claude Code. Skills and MCP configuration
remain separate Resources but share one Target, coordination barrier, restart, and readiness proof.

The JSON/JSONC adapter edits only server keys proven by `.ora-mcp-managed.json`. It preserves user
fields, comments, and unowned servers; name collisions, drift, higher-priority OpenCode JSONC, or
mismatched ownership block the complete Resource. Prepared operations remain secret-free and use
the existing Effect journal/recovery state machine.

Setting values stay under Ora's Plugin Configuration storage. Project files contain deterministic
environment references. When an Agent asks the host to spawn its CLI, Ora verifies the exact Agent,
Workspace, sidecar, rendered server fingerprint, installed contribution, and configuration revision
before injecting the referenced values into that child process. The plugin process itself cannot
set or receive `ORA_MCP_*` variables.

The desktop status endpoint accepts either an opaque Target id or a `(Workspace, Agent plugin)`
selector. Chat gates on the complete Target's desired/ready watermarks and blocking Conditions;
a running Agent process alone is insufficient.

The Plugin Configuration editor reports an MCP-wide aggregate: `Incomplete` when required Settings
are missing, `Projecting` while any active Agent Target is converging, `Current` when every matching
Target is ready at its desired generation, and `Blocked` when blocking or recovery evidence exists.
The editor polls only while the aggregate is `Projecting`.
