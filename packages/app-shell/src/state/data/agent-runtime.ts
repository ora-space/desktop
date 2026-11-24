import type { DiscoverAgentResponse } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";

/** What a discovery is made for; only one that needs commands may wait for an agent to report them. */
export type AgentDiscoveryScope = "models" | "modelsAndCommands";

/** Cache identity owned by agent-runtime data; consumers never repeat its tuples. */
export const agentRuntimeKeys = {
  agentRuntimeStatus: ["agentRuntimeStatus"] as const,
  /**
   * Scopes are kept apart because an answer without commands must not stand in for one that could
   * have had them: it would hide them for as long as it stays fresh.
   */
  agentDiscovery: (
    agentRef: string | null,
    workspaceId: string | null,
    scope: AgentDiscoveryScope,
  ) =>
    [
      "agentDiscovery",
      agentRef ?? "none",
      workspaceId ?? "none",
      scope,
    ] as const,
  agentDiscoveryForAgent: (agentRef: string) =>
    ["agentDiscovery", agentRef] as const,
};

/** Refreshes availability without probing a potentially stopped runtime for models. */
export function invalidateAgentAvailability(queryClient: QueryClient) {
  return queryClient.invalidateQueries({
    queryKey: agentRuntimeKeys.agentRuntimeStatus,
  });
}

/** Refreshes everything discovered from one agent in any workspace, leaving other agents untouched. */
export function invalidateAgentModels(
  queryClient: QueryClient,
  agentRef: string,
) {
  return queryClient.invalidateQueries({
    queryKey: agentRuntimeKeys.agentDiscoveryForAgent(agentRef),
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

/** Finds a chat's discovery of the same agent and workspace that nothing has outdated since. */
export function reusableChatDiscovery(
  queryClient: QueryClient,
  agentRef: string,
  workspaceId: string | null,
) {
  const state = queryClient.getQueryState<DiscoverAgentResponse>(
    agentRuntimeKeys.agentDiscovery(agentRef, workspaceId, "modelsAndCommands"),
  );
  return state?.isInvalidated === false && state.data !== undefined
    ? state
    : undefined;
}
