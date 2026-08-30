---
status: superseded by ADR-0015
---

# Gate Workspace Sessions on shared Effect readiness

This stronger all-entry admission design is superseded for the first release by ADR-0015. P1 requires a truthful Ready state and creates the end-to-end conversation only after Ready, but does not gate every warm, load, workflow, and prompt path on a combined Skill/MCP generation.

Ora keeps the existing application-level shared Agent process for the minimum closed loop. Workspace creation, Skill/MCP source changes, plugin configuration changes, and Agent capability registration update Desired State or wake the existing Effect worker; only that worker materializes the combined Skill and MCP generation. `session/new` never writes configuration and never runs a second MCP-specific reconcile path.

The Agent runtime centralizes one readiness gate across `session/new`, `session/load`, warm Session creation and reuse, interactive prompt admission, and workflow Agent nodes. Each path waits until its Workspace × Agent Target's combined generation is Ready. This closes the current first-Session race for Skills as well as MCPs and prevents an already-created or warmed Session from bypassing a newly pending generation. The wait is user-cancellable and has no arbitrary timeout; a deterministic conflict fails immediately, while Ora never bypasses the barrier to create or use a Session without its declared generation. Because the process-wide `agent/start` runs in a neutral home directory rather than one authoritative Workspace, it is not a valid Workspace materialization boundary.

When a Contract v2 Agent registers, Ora follows the existing Skill convergence policy by creating its target surfaces for every local Workspace and waking the worker even when no Session exists. Once later generations are accepted for reconciliation, Ora stops admitting new turns to that Agent, lets active turns finish, configures every due Workspace target, and performs one batched process restart rather than restarting once per Workspace. Successful targets advance independently; a conflicting target remains blocked at its previous generation and does not prevent the restart that activates successful siblings. The restart invalidates every live and warm provider Session across every Workspace using the shared Agent, and each re-establishment passes its own Target gate. Setting saves return after persistence and enqueue rather than waiting for this shared barrier. The design accepts this cross-Workspace impact instead of introducing a per-Workspace process model now.
