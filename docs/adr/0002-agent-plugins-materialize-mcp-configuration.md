---
status: accepted
---

# Agent Plugins materialize MCP configuration

Ora resolves installed MCP declarations into an Agent-independent complete set, while each MCP-capable Agent Plugin owns translation into its Agent's native schema. The first-release adapter is deliberately constrained: it renders one complete Ora-owned file and returns its bytes, locator, and digest; the Host validates and atomically applies the file. The Agent Plugin does not receive arbitrary Workspace write access and does not merge an existing user-owned file.

The first complete adapter is OpenCode and the first end-to-end MCP is Tavily. This keeps OpenCode's native schema outside Ora core without introducing the multi-artifact Plan/Apply/Observe protocol or user-file merger that ADR-0015 defers.
