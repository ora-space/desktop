import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidatePluginQueries } from "../data/plugins";
import { invalidateWorkflowQueries } from "../data/workflows";

/**
 * Imports one local `.orax` plugin archive selected by the user and refreshes
 * the installed and available surfaces once the backend settles.
 *
 * A package may also carry workflow documents that the backend turns into published workflows,
 * so workflow caches refresh alongside the plugin ones. That is deliberately not conditional on
 * the response: the import can install the package and still fail before reporting, and a stale
 * workflow library is worse than a refetch that usually finds nothing mounted to refresh.
 */
export function usePluginImport() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ path }: { path: string }) => client.plugin.import({ path }),
    onSettled: () =>
      Promise.all([
        invalidatePluginQueries(queryClient),
        invalidateWorkflowQueries(queryClient),
      ]),
  });
}
