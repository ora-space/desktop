import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/**
 * Imports one local `.orax` plugin archive selected by the user and refreshes
 * the installed and available surfaces once the backend settles.
 */
export function usePluginImport() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins }),
      queryClient.invalidateQueries({ queryKey: queryKeys.availablePlugins }),
    ]);

  return useMutation({
    mutationFn: ({ path }: { path: string }) => client.plugin.import({ path }),
    onSettled: invalidate,
  });
}
