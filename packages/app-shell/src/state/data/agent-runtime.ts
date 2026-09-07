import type { QueryClient } from "@tanstack/react-query";

/** Cache identity owned by agent-runtime data; consumers never repeat its tuples. */
export const agentRuntimeKeys = {
  agentRuntimeStatus: ["agentRuntimeStatus"] as const,
  agentModels: (agentRef: string | null, workspaceId: string | null) =>
    ["agentModels", agentRef ?? "none", workspaceId ?? "none"] as const,
  agentModelsForAgent: (agentRef: string) => ["agentModels", agentRef] as const,
};

/** Refreshes availability without probing a potentially stopped runtime for models. */
export function invalidateAgentAvailability(queryClient: QueryClient) {
  return queryClient.invalidateQueries({
    queryKey: agentRuntimeKeys.agentRuntimeStatus,
  });
}

/** Refreshes all workspace model projections for one agent, leaving other agents untouched. */
export function invalidateAgentModels(
  queryClient: QueryClient,
  agentRef: string,
) {
  return queryClient.invalidateQueries({
    queryKey: agentRuntimeKeys.agentModelsForAgent(agentRef),
  });
}

/** Only a starting agent can answer discovery; stop/removal must not restart model probes. */
export function refreshAgent(
  queryClient: QueryClient,
  agentRef: string,
  scope: "availability" | "models",
) {
  const requests = [invalidateAgentAvailability(queryClient)];
  if (scope === "models")
    requests.push(invalidateAgentModels(queryClient, agentRef));
  return Promise.all(requests);
}
