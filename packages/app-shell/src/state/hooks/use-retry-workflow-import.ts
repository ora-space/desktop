import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidatePluginQueries } from "../data/plugins";
import { invalidateWorkflowQueries } from "../data/workflows";

/**
 * Retries the workflow document import of one installed plugin.
 *
 * This is the card action, not a second install: the host reconsiders only the document paths that
 * have no terminal record, so already imported and name-conflict-skipped paths stay untouched. A
 * retry that creates workflows refreshes both the plugin surface and the workflow library, because
 * the card's summary and the library list are read from different queries.
 */
export function useRetryWorkflowImport(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => client.plugin.retryWorkflowImport({ pluginId }),
    onSettled: async () => {
      await Promise.all([
        invalidatePluginQueries(queryClient),
        invalidateWorkflowQueries(queryClient),
      ]);
    },
  });
}
