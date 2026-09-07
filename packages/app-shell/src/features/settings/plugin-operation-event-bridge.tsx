import { useEffect } from "react";
import { usePlatform } from "../../platform";
import { usePluginOperationStore } from "../../state/stores/plugin-operation-store";

/** Keeps native plugin transfer progress alive when the settings dialog or plugin page unmounts. */
export function PluginOperationEventBridge() {
  const { pluginMarketplace } = usePlatform();

  useEffect(() => {
    if (pluginMarketplace === undefined) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void pluginMarketplace
      .onInstallProgress((progress) => {
        if (!disposed) {
          usePluginOperationStore.getState().reportTransferProgress(progress);
        }
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [pluginMarketplace]);

  return null;
}
