import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/**
 * Installs one marketplace plugin and refreshes the installed and available
 * surfaces once the backend settles. The optional `signal` lets the caller
 * cancel the pending request; the installed lookup is refreshed either way.
 */
export function useInstallPlugin(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins }),
      queryClient.invalidateQueries({ queryKey: queryKeys.availablePlugins }),
    ]);

  return useMutation({
    mutationFn: ({ signal }: { signal?: AbortSignal } = {}) =>
      client.plugin.install({ pluginId }, { signal }),
    onSettled: invalidate,
  });
}
