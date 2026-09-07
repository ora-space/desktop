import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidateAvailablePlugins } from "../data/plugins";

/** Pulls the marketplace source, rebuilds its index, and refreshes the available plugin query. */
export function usePluginRegistrySync() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => client.plugin.syncAvailable({}),
    onSettled: () => invalidateAvailablePlugins(queryClient),
  });
}
