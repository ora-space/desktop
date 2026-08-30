---
status: accepted
---

# Automatically enable every Ready MCP globally in the first release

Ora automatically includes every configuration-ready MCP in every local Workspace instead of requiring per-Workspace selection, because installation plus completed configuration should make the capability immediately useful. The first release has no Workspace exclusion: uninstalling the MCP or removing required configuration removes it from the global effective set. Incomplete MCPs remain in `NeedsConfiguration` rather than blocking unrelated Agent use. Existing local Workspaces are eagerly given an MCP Effect surface once an MCP-capable Agent registers; this accepts a global blast radius and no per-project opt-out in exchange for the smallest usable setup and state model.

Package-retention and asynchronous retirement rules are deliberately not decided here; ADR-0015 keeps them outside the first closed loop.
