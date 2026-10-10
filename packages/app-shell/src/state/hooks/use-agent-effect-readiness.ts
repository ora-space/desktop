import { useQuery } from "@tanstack/react-query";
import type { GetEffectTargetStatusResponse } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { effectKeys } from "../data/effects";
import { useInstalledPlugins } from "./use-installed-plugins";

export type AgentEffectReadiness = "ready" | "blocked";

/** Cadence used only while the send gate is still waiting on Target evidence. */
const BLOCKED_POLL_INTERVAL_MS = 1_000;

type EffectTargetStatus = GetEffectTargetStatusResponse["status"];

/** True when persisted Target evidence says the first prompt may proceed. */
function isEffectTargetReady(status: EffectTargetStatus | undefined): boolean {
  if (status === undefined || status === null) return false;
  const current =
    status.phase === "current" || status.phase === "current_with_issues";
  const blocking = status.conditions.some(
    (condition) => condition.impact === "blocking",
  );
  return (
    current && status.readyGeneration >= status.desiredGeneration && !blocking
  );
}

/**
 * Polls only while chat is still gated. A ready Target is expected to stay
 * ready until workspace or agent identity changes (a new query key).
 */
export function effectTargetStatusPollInterval(
  status: EffectTargetStatus | undefined,
): number | false {
  return isEffectTargetReady(status) ? false : BLOCKED_POLL_INTERVAL_MS;
}

/** Gates chat on the complete persisted Effect Target, never on one Resource in isolation. */
export function useAgentEffectReadiness(
  workspaceId: string | undefined,
  agentRef: string | null,
): AgentEffectReadiness {
  const client = useContractsClient();
  const installedPlugins = useInstalledPlugins();
  const managedAgent =
    agentRef !== null &&
    (installedPlugins.data ?? []).some(
      (plugin) => plugin.kind === "agent" && plugin.id === agentRef,
    );
  const query = useQuery({
    queryKey: effectKeys.agentEffectStatus(workspaceId ?? "", agentRef ?? ""),
    queryFn: () =>
      client.effect.getTargetStatus({
        selector: "workspace_agent",
        workspaceId: workspaceId ?? "",
        agentPluginId: agentRef ?? "",
      }),
    enabled: managedAgent && workspaceId !== undefined,
    // Idle ready chats must not keep invoking get_effect_target_status every
    // second. Workspace or agent changes already create a new query key.
    refetchInterval: ({ state }) =>
      effectTargetStatusPollInterval(state.data?.status),
  });
  // The plugin snapshot determines whether this agent owns an Effect Target. Keep the first
  // prompt behind the gate until that local snapshot arrives, rather than treating loading as an
  // absent agent and letting a new Worktree race materialization.
  if (agentRef !== null && installedPlugins.isPending) return "blocked";
  if (!managedAgent || workspaceId === undefined) return "ready";
  const status = query.data?.status;
  // A first prompt must not race the initial status query: no Target evidence is not readiness.
  if (status === undefined) return "blocked";
  return isEffectTargetReady(status) ? "ready" : "blocked";
}
