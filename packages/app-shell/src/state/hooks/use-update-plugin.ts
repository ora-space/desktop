import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { queryKeys } from "./query-keys";

/**
 * Updates one installed marketplace plugin to the version its source publishes and refreshes
 * the installed and available surfaces once the backend settles. The optional `signal` lets the
 * caller cancel the pending request; the installed lookup is refreshed either way.
 */
export function useUpdatePlugin(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const activity = usePluginOperationStore(
    (state) => state.activities[pluginId],
  );
  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins }),
      queryClient.invalidateQueries({ queryKey: queryKeys.availablePlugins }),
    ]);

  const mutation = useMutation({
    mutationFn: ({ signal }: { signal?: AbortSignal } = {}) =>
      client.plugin.update({ pluginId }, { signal }),
    onSettled: async () => {
      try {
        await invalidate();
      } finally {
        usePluginOperationStore.getState().clear(pluginId);
      }
    },
  });
  const mutate = (...args: Parameters<typeof mutation.mutate>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "update")) return;
    mutation.mutate(...args);
  };
  const mutateAsync = (...args: Parameters<typeof mutation.mutateAsync>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "update")) {
      return Promise.reject(
        new Error(`plugin operation already pending: ${pluginId}`),
      );
    }
    return mutation.mutateAsync(...args);
  };

  return {
    ...mutation,
    isPending: activity?.state === "pending" && activity.kind === "update",
    mutate,
    mutateAsync,
    progress:
      activity?.state === "pending" && activity.kind === "update"
        ? activity.progress
        : null,
  };
}
