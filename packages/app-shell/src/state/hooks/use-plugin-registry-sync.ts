import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidateAvailablePlugins } from "../data/plugins";
import { useMarketplaceSyncStore } from "../stores/marketplace-sync-store";

/**
 * Pulls the marketplace source, rebuilds its index, and refreshes the available plugin query.
 *
 * The pending state is held in the marketplace sync store rather than in the mutation, because
 * the rebuild outlives the settings page: leaving it mid-sync and coming back would otherwise
 * restore a Sync button that looks ready while the host is still rebuilding.
 */
export function usePluginRegistrySync() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const syncing = useMarketplaceSyncStore((state) => state.userSyncing);

  const mutation = useMutation({
    mutationFn: () => client.plugin.syncAvailable({}),
    // Declared on the mutation rather than passed to `mutate`, so it still runs when the page
    // that started the sync has since unmounted. Nothing else would release the flag.
    onSettled: async () => {
      try {
        await invalidateAvailablePlugins(queryClient);
      } finally {
        useMarketplaceSyncStore.getState().setUserSyncing(false);
      }
    },
  });

  const mutate = (...args: Parameters<typeof mutation.mutate>) => {
    if (useMarketplaceSyncStore.getState().userSyncing) return;
    useMarketplaceSyncStore.getState().setUserSyncing(true);
    mutation.mutate(...args);
  };

  return { ...mutation, isPending: syncing, mutate };
}
