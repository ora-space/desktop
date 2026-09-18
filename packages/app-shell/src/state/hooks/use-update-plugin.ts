import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { invalidatePluginQueries } from "../data/plugins";

/** What one update request carries beyond the plugin it names. */
interface UpdatePluginVariables {
  /** Lets the caller cancel the pending request. */
  signal?: AbortSignal;
  /**
   * Declares that the user authorized running the package's Hook `init` command.
   *
   * Updates re-run `init` every time so the tool can migrate what an earlier version wrote, which
   * makes the disclosure a per-update one rather than a one-time consent at install.
   */
  hookExecutionAcknowledged?: boolean;
}

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
  const invalidate = () => invalidatePluginQueries(queryClient);

  const mutation = useMutation({
    mutationFn: ({
      signal,
      hookExecutionAcknowledged = false,
    }: UpdatePluginVariables = {}) =>
      client.plugin.update({ pluginId, hookExecutionAcknowledged }, { signal }),
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
