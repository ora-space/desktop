import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

export type AgentEffectReadiness = "ready" | "blocked" | "unknown";

/** Gates chat on the complete persisted Effect Target, never on one Resource in isolation. */
export function useAgentEffectReadiness(
  workspaceId: string | undefined,
  agentRef: string | null,
): AgentEffectReadiness {
  const client = useContractsClient();
  const managedAgent =
    agentRef === "ora-space.opencode" || agentRef === "ora-space.claude";
  const query = useQuery({
    queryKey: queryKeys.agentEffectStatus(workspaceId ?? "", agentRef ?? ""),
    queryFn: () =>
      client.effect.getTargetStatus({
        selector: "workspace_agent",
        workspaceId: workspaceId ?? "",
        agentPluginId: `official/${agentRef ?? ""}`,
      }),
    enabled: managedAgent && workspaceId !== undefined,
    refetchInterval: 1_000,
  });
  if (!managedAgent || workspaceId === undefined) return "ready";
  const status = query.data?.status;
  if (status === undefined) return "unknown";
  if (status === null) return "blocked";
  const current =
    status.phase === "current" || status.phase === "current_with_issues";
  const blocking = status.conditions.some(
    (condition) => condition.impact === "blocking",
  );
  return current &&
    status.readyGeneration >= status.desiredGeneration &&
    !blocking
    ? "ready"
    : "blocked";
}
