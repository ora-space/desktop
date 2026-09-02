import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";
import { useAgentRuntimeStatus } from "./use-agent-runtime-status";

/** Discovers one plugin-owned model catalog without creating an Ora provider session. */
export function useAgentModels(
  agentRef: string | null,
  workspaceId: string | null,
) {
  const client = useContractsClient();
  const { data: statuses } = useAgentRuntimeStatus();
  const runtimeStatus = statuses?.find(
    (status) => status.agentRef === agentRef,
  )?.status;
  const ready = runtimeStatus === "ready";
  const query = useQuery({
    queryKey: queryKeys.agentModels(agentRef, workspaceId),
    queryFn: () =>
      client.agentRuntime.listModels({
        agentRef: agentRef!,
        workspaceId: workspaceId!,
      }),
    enabled: agentRef !== null && workspaceId !== null && ready,
    retry: false,
    // Discovery is a real plugin call that may start a one-shot agent process,
    // so navigating between chat surfaces must not re-run it. What makes a
    // catalog stale is an event — an agent process replaced, a plugin installed,
    // enabled, or removed — and each of those invalidates this key explicitly.
    staleTime: 5 * 60 * 1000,
  });
  return {
    ...query,
    models: query.data?.models ?? [],
    isLoading:
      query.isLoading ||
      (agentRef !== null &&
        workspaceId !== null &&
        (statuses === undefined || runtimeStatus === "starting")),
  };
}
