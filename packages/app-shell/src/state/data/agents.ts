import type { QueryClient } from "@tanstack/react-query";

/** Cache identity owned by agents data; consumers never repeat its tuples. */
export const agentKeys = {
  agents: ["agents"] as const,
};

/** Refreshes the shared agent-definition list after imports or CRUD. */
export function invalidateAgents(queryClient: QueryClient) {
  return queryClient.invalidateQueries({ queryKey: agentKeys.agents });
}
