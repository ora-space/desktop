import type {
  ContractsClient,
  GetEffectTargetStatusResponse,
} from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";

const agentEffectStatuses = ["agent-effect-status"] as const;

/** Cache identity owned by effects data; consumers never repeat its tuples. */
export const effectKeys = {
  agentEffectStatuses,
  agentEffectStatus: (workspaceId: string, agentRef: string) =>
    [...agentEffectStatuses, workspaceId, agentRef] as const,
};

/** Refreshes only cached selectors that observed the changed Target. */
async function refreshEffectTargetStatus(
  queryClient: QueryClient,
  targetId: string,
) {
  // Target identity comes from authoritative status responses. Multiple selectors may
  // observe the same Target; other workspaces and agents keep their readiness caches.
  const selectors = queryClient.getQueriesData<GetEffectTargetStatusResponse>({
    queryKey: effectKeys.agentEffectStatuses,
  });
  await Promise.all(
    selectors
      .filter(([, data]) => data?.status?.targetId === targetId)
      .map(([queryKey]) =>
        queryClient.invalidateQueries({ queryKey, exact: true }),
      ),
  );
}

/** Recovers one Target and refreshes only cached selectors that observed that Target. */
export async function recoverEffectTarget(
  client: ContractsClient,
  queryClient: QueryClient,
  targetId: string,
) {
  const response = await client.effect.recoverTarget({ targetId });
  await refreshEffectTargetStatus(queryClient, targetId);
  return response;
}

/** Wakes one Target after external cleanup and refreshes only its cached selectors. */
export async function retryEffectTarget(
  client: ContractsClient,
  queryClient: QueryClient,
  targetId: string,
) {
  const response = await client.effect.retryTarget({ targetId });
  await refreshEffectTargetStatus(queryClient, targetId);
  return response;
}
