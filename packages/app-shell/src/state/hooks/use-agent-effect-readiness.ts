import { useQuery } from "@tanstack/react-query";
import type { GetEffectTargetStatusResponse } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { effectKeys } from "../data/effects";
import { useInstalledPlugins } from "./use-installed-plugins";

/** Send-gate state derived from one complete persisted Effect Target. */
export type AgentEffectReadiness =
  { kind: "ready" } | { kind: "blocked" } | { kind: "failed"; message: string };

/** Cadence used only while the send gate is still waiting on Target evidence. */
const BLOCKED_POLL_INTERVAL_MS = 1_000;

/**
 * Manual-recovery states do not resolve on their own, so a slower cadence keeps the gate
 * eventually consistent without per-second polling that cannot change the outcome.
 */
const FAILED_POLL_INTERVAL_MS = 10_000;

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
 * Returns the reason a Target needs manual intervention instead of another sync interval,
 * or null while convergence is still expected to make progress on its own.
 */
export function effectTargetFailureMessage(
  status: EffectTargetStatus | undefined,
): string | null {
  if (status === undefined || status === null) return null;
  const manualBlocking = status.conditions.find(
    (condition) =>
      condition.impact === "blocking" && condition.retry === "manual",
  );
  if (status.phase !== "recovery_required" && manualBlocking === undefined) {
    return null;
  }
  return manualBlocking?.message ?? "Effect recovery is required.";
}

/**
 * Polls only while chat is still gated. A ready Target is expected to stay
 * ready until workspace or agent identity changes (a new query key).
 */
export function effectTargetStatusPollInterval(
  status: EffectTargetStatus | undefined,
): number | false {
  if (isEffectTargetReady(status)) return false;
  if (effectTargetFailureMessage(status) !== null) {
    return FAILED_POLL_INTERVAL_MS;
  }
  return BLOCKED_POLL_INTERVAL_MS;
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
  // prompt behind the gate until that local snapshot arrives, rather than treating loading as
  // an absent agent and letting a new Worktree race materialization.
  if (agentRef !== null && installedPlugins.isPending)
    return { kind: "blocked" };
  if (!managedAgent || workspaceId === undefined) return { kind: "ready" };
  const status = query.data?.status;
  // A first prompt must not race the initial status query: no Target evidence is not readiness.
  if (status === undefined) return { kind: "blocked" };
  if (isEffectTargetReady(status)) return { kind: "ready" };
  const failure = effectTargetFailureMessage(status);
  if (failure !== null) return { kind: "failed", message: failure };
  return { kind: "blocked" };
}
