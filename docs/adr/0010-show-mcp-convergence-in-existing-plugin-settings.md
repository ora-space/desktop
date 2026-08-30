---
status: accepted
---

# Show MCP convergence in existing Plugin Settings

The first release adds no Workspace MCP configuration page. The existing Settings → Plugins surface shows each MCP Plugin's derived application state: `NeedsConfiguration`, `WaitingForAgent`, `Applying`, `Ready`, or `Failed`. Complete Settings without a registered `agent_mcp_v1` consumer are `WaitingForAgent`, because configuration completeness does not prove that an Agent can use the MCP. Registration immediately creates or refreshes eager local-Workspace surfaces; `Ready` is shown only after every registered local consumer surface has applied the current file and completed Agent activation. Failures report the affected Workspace and safe conflict paths without sensitive values.

The state is a read model over existing configuration, surface, operation, and consumer rows. It does not add an Agent Target table or a Workspace MCP settings page.
