---
status: superseded by ADR-0015
---

# Retire MCPs before deleting packages or settings

The full asynchronous retirement and retention state machine is outside the first closed loop selected in ADR-0015. P1 must converge the current effective set and safely remove its own file when the set becomes empty, but does not block package or Agent uninstallation on a cross-Workspace retirement workflow.

Uninstalling an MCP or resetting a required Setting first removes it from every Workspace's desired generation and waits for Agent Targets to remove their managed entries and exact-version references. Only then may Ora delete the installed package or selected plugin data; retained data makes a later reinstall Ready and globally enabled again. An ownership or drift conflict leaves the MCP in an explicit `Retiring / Blocked` state and retains both package and values until the user follows the reported manual remediation and reconciliation completes. Ora never force-deletes the retained resources while leaving an unverifiable Agent-native entry behind.

The same ordering applies when uninstalling an MCP-capable Agent Plugin. Ora first uses the still-installed adapter to retire all Agent Targets and remove their Managed MCP entries and sidecars. A cleanup conflict blocks Agent package deletion, because deleting the only adapter that understands those targets would make safe retirement impossible. These asynchronous retirement rules prevent live configuration from referring to deleted executables, stale credentials, or an adapter that no longer exists.

These reference-retention rules continue to govern MCP package versions. The separate first-release decision to keep the current non-transactional OpenCode Agent updater does not authorize deletion of an exact MCP version while `effect_managed_items` still references it.
