import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { usePlatform } from "../../platform";
import { invalidateAvailablePlugins } from "../../state/data/plugins";
import { useMarketplaceSyncStore } from "../../state/stores/marketplace-sync-store";

/**
 * Keeps the shell in step with the marketplace refreshes the host runs on its own.
 *
 * Mounted at the shell root rather than inside the settings dialog: a refresh runs shortly after
 * launch and every six hours after that, long before anyone opens the plugin page, and its result
 * has to reach the cached listing either way.
 */
export function MarketplaceAutoSyncBridge() {
  const { pluginMarketplace } = usePlatform();
  const queryClient = useQueryClient();

  useEffect(() => {
    if (pluginMarketplace === undefined) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void pluginMarketplace
      .onAutoSyncChanged((event) => {
        if (disposed) return;
        useMarketplaceSyncStore
          .getState()
          .setHostRefreshing(event.kind === "started");
        if (event.kind === "finished")
          void invalidateAvailablePlugins(queryClient);
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unsubscribe?.();
      // A refresh that outlives this subscription would otherwise leave the Sync action disabled
      // for good, since its `finished` event can no longer arrive.
      useMarketplaceSyncStore.getState().setHostRefreshing(false);
    };
  }, [pluginMarketplace, queryClient]);

  return null;
}
