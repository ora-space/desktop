import type { AgentCli } from "@ora/contracts";
import { useSettingsStore } from "../stores/settings-store";
import { usePendingAgentStore } from "../stores/pending-agent-store";
import { warmTargetKey } from "./use-warm-session";

/**
 * Resolves which agent a chat surface is currently set to.
 *
 * A started chat already has its agent recorded on the session itself, so this
 * is just the shared default once one exists — moving it onto another CLI is
 * `switchSessionAgent`'s job, not this hook's. Before a session exists there is
 * nowhere else to record the pick, and the shared default alone cannot: reading
 * it directly would let picking an agent for one not-yet-started chat repaint
 * every other one the moment it is visited. So this prefers whatever was last
 * picked for this exact target, held in `usePendingAgentStore`, and only falls
 * back to the shared default for a target no one has touched yet.
 *
 * Callers that also warm a session for this target must resolve the agent this
 * same way, so the session that gets created is the one the picker is showing.
 */
export function useTargetAgentCli(selection: {
  projectId: string | null;
  taskId: string | null;
  sessionId: string | null;
}): AgentCli {
  const defaultAgentCli = useSettingsStore((state) => state.settings.agentCli);
  const targetKey = warmTargetKey(selection);
  const pendingAgentCli = usePendingAgentStore((state) =>
    targetKey === null ? undefined : state.selections[targetKey],
  );
  if (selection.sessionId !== null) return defaultAgentCli;
  return pendingAgentCli ?? defaultAgentCli;
}
