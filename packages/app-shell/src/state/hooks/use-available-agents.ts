import { useMemo } from "react";
import {
  useAgentCatalog,
  type AgentEntry,
} from "../../features/chat/agent-catalog";
import { useAgentRuntimeStatus } from "./use-agent-runtime-status";

/**
 * Resolves which agents the pickers may offer on this installation, in catalog order.
 *
 * Which agents exist is not knowable at build time. An agent is supplied by an installed plugin
 * and does not exist here at all unless that package is installed, and the user can revoke it at
 * any time by disabling the package or by the agent process behind it going missing. Offering an
 * agent in any of those states advertises something no session could be opened on, so the list is
 * the intersection of the installed catalog with the agents the runtime actually reports reaching.
 *
 * `starting` counts alongside `ready` because an agent still completing its handshake is on its
 * way to being usable, and an entry that vanished for the first second of every launch would take
 * the surfaces pointing at it along with it. `unavailable` and `failing` do not: the first means
 * nothing answered — a disabled package, an agent process that is not installed — and the second
 * means the supervisor has given up on it for the rest of the process.
 *
 * While the status is still loading the full catalog is offered. "Not answered yet" is not "not
 * detected", and answering it as such would move every surface onto a different agent for as long
 * as that query is in flight.
 */
export function useAvailableAgents(): AgentEntry[] {
  const catalog = useAgentCatalog();
  const { data: statuses } = useAgentRuntimeStatus();
  return useMemo(() => {
    if (statuses === undefined) return catalog;
    const detected = new Set(
      statuses
        .filter(
          (status) => status.status === "ready" || status.status === "starting",
        )
        .map((status) => status.agentRef),
    );
    return catalog.filter((entry) => detected.has(entry.agentRef));
  }, [catalog, statuses]);
}
