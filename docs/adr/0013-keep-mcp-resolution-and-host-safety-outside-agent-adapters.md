---
status: accepted
---

# Keep MCP resolution and Host safety outside Agent adapters

`ora-plugin-config` owns a pure, Agent-independent resolver from a compiled MCP declaration and an exact matching plugin-store revision into `ResolvedMcp` or `NeedsConfiguration`. It does not depend on SQLite, Effect, Agent runtime, or OpenCode, which keeps Setting-expression behavior directly unit-testable. P1 resolves only the HTTP profile needed by Tavily; the already compiled stdio syntax does not imply stdio materialization support.

Effect stores a typed logical `agent_mcp_v1` surface rather than treating the Workspace root as a generic mutable filesystem surface. The Agent adapter may render only the single file authorized by that surface. The Host independently validates the fixed locator, local-Workspace containment, link/reparse safety, existing-file ownership proof, Git local-exclude precondition, size, and digest before atomic replacement. Existing user-owned target files fail closed; because P1 never merges them, it does not need a general Plan CAS or per-entry fingerprint protocol.
