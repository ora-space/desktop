import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { invalidatePluginQueries } from "../data/plugins";

/** What one install request carries beyond the plugin it names. */
interface InstallPluginVariables {
  /** Lets the caller cancel the pending request. */
  signal?: AbortSignal;
  /**
   * Declares that the user authorized running the package's Hook `init` command.
   *
   * Defaults to withholding execution: a Hook only runs when a caller that showed the user the
   * execution disclosure asks for it, so a missed wiring step installs a dormant package instead
   * of running a program nobody was warned about.
   */
  hookExecutionAcknowledged?: boolean;
}

/**
 * Installs one marketplace plugin and refreshes the installed and available
 * surfaces once the backend settles. The optional `signal` lets the caller
 * cancel the pending request; the installed lookup is refreshed either way.
 */
export function useInstallPlugin(pluginId: string) {
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
    }: InstallPluginVariables = {}) =>
      client.plugin.install(
        { pluginId, hookExecutionAcknowledged },
        { signal },
      ),
    onSettled: async (_response, error) => {
      try {
        await invalidate();
      } finally {
        const operations = usePluginOperationStore.getState();
        if (error === null) operations.completeInstall(pluginId);
        else operations.clear(pluginId);
      }
    },
  });

  const mutate = (...args: Parameters<typeof mutation.mutate>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "install")) return;
    mutation.mutate(...args);
  };
  const mutateAsync = (...args: Parameters<typeof mutation.mutateAsync>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "install")) {
      return Promise.reject(
        new Error(`plugin operation already pending: ${pluginId}`),
      );
    }
    return mutation.mutateAsync(...args);
  };

  const installing =
    activity?.state === "pending" && activity.kind === "install";
  const progress = installing ? activity.progress : null;
  const completionId =
    activity?.state === "install_completed" ? activity.completionId : null;
  const consumeCompletion = () => {
    if (completionId !== null) {
      usePluginOperationStore
        .getState()
        .consumeInstallCompletion(pluginId, completionId);
    }
  };
  return {
    ...mutation,
    isPending: installing,
    isSuccess: mutation.isSuccess || activity?.state === "install_completed",
    mutate,
    mutateAsync,
    progress,
    completionId,
    consumeCompletion,
  };
}
