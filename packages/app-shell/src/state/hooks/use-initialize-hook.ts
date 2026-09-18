import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { invalidateHookLifecycleReports } from "../data/plugins";

/**
 * Runs one installed Hook package's declared `init` command on the user's explicit request.
 *
 * This is both the retry path for a failed `init` and the only way a Hook the host never executed
 * for — a pack member, for instance — is initialized: the click is the authorization the request
 * carries, so the store admits one initialization per plugin at a time like every other lifecycle
 * action.
 */
export function useInitializeHook(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const activity = usePluginOperationStore(
    (state) => state.activities[pluginId],
  );

  const mutation = useMutation({
    mutationFn: () =>
      client.plugin.initializeHook({
        pluginId,
        hookExecutionAcknowledged: true,
      }),
    onSettled: async () => {
      try {
        await invalidateHookLifecycleReports(queryClient);
      } finally {
        usePluginOperationStore.getState().clear(pluginId);
      }
    },
  });
  const mutate = (...args: Parameters<typeof mutation.mutate>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "initialize"))
      return;
    mutation.mutate(...args);
  };

  return {
    ...mutation,
    isPending: activity?.state === "pending" && activity.kind === "initialize",
    mutate,
  };
}
