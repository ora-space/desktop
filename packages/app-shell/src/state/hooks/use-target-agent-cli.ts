import type { AgentCli } from "@ora/contracts";
import { useSettingsStore } from "../stores/settings-store";
import { usePendingAgentStore } from "../stores/pending-agent-store";
import { warmTargetKey } from "./use-warm-session";

/**
 * Resolves which agent a chat surface warms against before it has a binding.
 *
 * This answers only for a surface the backend has no session row for. A
 * persisted session runs on whichever CLI it is bound to, and that binding —
 * not this — is what the picker must show for it; moving it is
 * `switchSessionAgent`'s job. Callers layer the binding over this result rather
 * than this hook guessing at it.
 *
 * Before a row exists there is nowhere else to record the pick, and the shared
 * default alone cannot hold it: reading that directly would let picking an
 * agent for one not-yet-started chat repaint every other one the moment it is
 * visited. So this prefers whatever was last picked for this exact target, held
 * in `usePendingAgentStore`, and falls back to the shared default only for a
 * target no one has touched yet.
 *
 * Keyed the same way `useWarmSession` keys its target, so a surface always warms
 * against the agent the picker is showing for it.
 */
export function useTargetAgentCli(selection: {
  projectId: string | null;
  taskId: string | null;
}): AgentCli {
  const defaultAgentCli = useSettingsStore((state) => state.settings.agentCli);
  const targetKey = warmTargetKey(selection);
  const pickedForTarget = usePendingAgentStore((state) =>
    targetKey === null ? undefined : state.selections[targetKey],
  );
  return pickedForTarget ?? defaultAgentCli;
}
