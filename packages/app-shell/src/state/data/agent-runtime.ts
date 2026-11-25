import type { DiscoverAgentResponse } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";

/** What a discovery is made for; only one that needs commands may wait for an agent to report them. */
export type AgentDiscoveryScope = "models" | "modelsAndCommands";

const modelsScope: AgentDiscoveryScope = "models";
const commandsScope: AgentDiscoveryScope = "modelsAndCommands";

/** Cache identity owned by agent-runtime data; consumers never repeat its tuples. */
export const agentRuntimeKeys = {
  agentRuntimeStatus: ["agentRuntimeStatus"] as const,
  agentDiscoveries: ["agentDiscovery"] as const,
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

/**
 * Refreshes everything discovered from one agent in any workspace, leaving other agents untouched.
 *
 * Requests already running are abandoned first. Invalidating alone leaves a first one running, and
 * its answer, made before the change, would then count as fresh.
 */
export function invalidateAgentModels(
  queryClient: QueryClient,
  agentRef: string,
) {
  const filters = {
    queryKey: agentRuntimeKeys.agentDiscoveryForAgent(agentRef),
  };
  void queryClient.cancelQueries(filters);
  return queryClient.invalidateQueries(filters);
}

/**
 * Refreshes the commands discovered from one agent in any workspace, which depend on Skills.
 *
 * Its models do not, so answers given for models alone stand, and so do the models of those that
 * came with commands: they are kept for whoever asks for models only.
 */
export function invalidateAgentCommands(
  queryClient: QueryClient,
  agentRef: string,
) {
  const filters = {
    queryKey: agentRuntimeKeys.agentDiscoveryForAgent(agentRef),
    predicate: ({ queryKey }: { queryKey: readonly unknown[] }) =>
      queryKey.at(-1) === commandsScope,
  };
  for (const { queryKey, state } of queryClient
    .getQueryCache()
    .findAll(filters)) {
    const discovered = state.data as DiscoverAgentResponse | undefined;
    if (discovered === undefined || state.isInvalidated) continue;
    const modelsKey = [...queryKey.slice(0, -1), modelsScope];
    const kept = queryClient.getQueryState(modelsKey);
    if (
      kept !== undefined &&
      !kept.isInvalidated &&
      kept.dataUpdatedAt >= state.dataUpdatedAt
    )
      continue;
    queryClient.setQueryData<DiscoverAgentResponse>(
      modelsKey,
      { models: discovered.models },
      { updatedAt: state.dataUpdatedAt },
    );
  }
  void queryClient.cancelQueries(filters);
  return queryClient.invalidateQueries(filters);
}

/** Refreshes the commands discovered from every agent, for changes nobody reported one by one. */
export function invalidateAllAgentCommands(queryClient: QueryClient) {
  const agents = queryClient
    .getQueryCache()
    .findAll({ queryKey: agentRuntimeKeys.agentDiscoveries })
    .map(({ queryKey }) => String(queryKey[1]));
  return Promise.all(
    [...new Set(agents)].map((agentRef) =>
      invalidateAgentCommands(queryClient, agentRef),
    ),
  );
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
