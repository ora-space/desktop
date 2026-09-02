import { useAgentRuntimeStatus } from "./use-agent-runtime-status";
import { useTargetAgentCli, type AgentSelection } from "./use-target-agent-cli";

/**
 * The send-gate verdict for one chat surface, kept distinct from
 * `useAvailableAgents`' rule: that list offers `starting` agents in the picker
 * because they are on their way to being usable, but first send still rejects
 * them, and a gate that offers a send the backend would refuse
 * is the bug this exists to prevent.
 *
 * - `"ready"` — the runtime reports this CLI ready, so a send can go out.
 * - `"blocked"` — detection answered and this CLI is not ready: starting,
 *   unavailable, failing, or absent from the report. Exactly the states where a
 *   send would fail. A surface that never resolved a CLI lands here too, which
 *   is harmless — the whole-composer disable already owns that state.
 * - `"unknown"` — detection has not answered; the button stays as-is rather
 *   than guessing.
 */
export type AgentReadiness = "ready" | "blocked" | "unknown";

/**
 * Whether a chat surface's resolved agent can actually carry a send.
 *
 * First send refuses to open a provider session unless the runtime
 * reports `ready`, so a composer pointing anywhere else fails its send with a
 * raw error instead of preventing it. This derives that same answer for UI, and
 * deliberately reuses `useTargetAgentCli` rather than re-deriving any leg of the
 * precedence chain: a picker and a gate that resolved the CLI differently would
 * offer an agent the send button refuses.
 */
export function useTargetAgentReadiness(
  selection: AgentSelection,
): AgentReadiness {
  const agentCli = useTargetAgentCli(selection);
  const { data: statuses } = useAgentRuntimeStatus();
  // Detection has not answered yet — the query is still in flight or it failed.
  // Gating here would freeze the send button on every startup and forever after
  // a failed status fetch, for an answer nobody has.
  if (statuses === undefined) return "unknown";
  // A single scan answers both "which status does this CLI have" and "does the
  // CLI exist in the report at all", so an agent that dropped out of detection
  // reads the same as one that is explicitly down.
  return statuses.some(
    (status) => status.agentRef === agentCli && status.status === "ready",
  )
    ? "ready"
    : "blocked";
}
