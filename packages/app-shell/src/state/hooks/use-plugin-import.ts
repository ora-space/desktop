import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidatePluginQueries } from "./plugin-invalidation";

/**
 * Imports one local `.orax` plugin archive selected by the user and refreshes
 * the installed and available surfaces once the backend settles.
 */
export function usePluginImport() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ path }: { path: string }) => client.plugin.import({ path }),
    onSettled: () => invalidatePluginQueries(queryClient),
  });
}
