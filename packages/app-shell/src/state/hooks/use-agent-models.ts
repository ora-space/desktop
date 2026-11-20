import { useQuery, useQueryClient } from "@tanstack/react-query";
import { RemoteContractError } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { agentRuntimeKeys } from "../data/agent-runtime";
import { refreshWorkspaceDirectories } from "../data/workspace";
import { useAgentRuntimeStatus } from "./use-agent-runtime-status";

/** Discovers one plugin-owned model catalog without creating an Ora provider session. */
export function useAgentModels(
  agentRef: string | null,
  workspaceId: string | null,
) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const { data: statuses } = useAgentRuntimeStatus();
  const runtimeStatus = statuses?.find(
    (status) => status.agentRef === agentRef,
  )?.status;
  const ready = runtimeStatus === "ready";
  const query = useQuery({
    queryKey: agentRuntimeKeys.agentModels(agentRef, workspaceId),
    queryFn: () =>
      client.agentRuntime
        .listModels({
          agentRef: agentRef!,
          workspaceId: workspaceId!,
        })
        .catch((error: unknown) => {
          // Discovery resolves the workspace folder before asking the plugin. A refusal here
          // usually means the folder vanished after the workspace list last probed it, so the
          // list is refreshed and the composer can name the missing folder instead of showing
          // an agent with no models.
          if (
            error instanceof RemoteContractError &&
            error.code === "workspace_unavailable"
          ) {
            void refreshWorkspaceDirectories(queryClient);
          }
          throw error;
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
