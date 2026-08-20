import { useEffect } from "react";
import { usePlatform } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";

/**
 * Hydrates the surface store from the host and streams lifecycle events into it.
 *
 * Download events are left to `SurfaceDownloadToaster`; the store only mirrors
 * instance state. Mounted once in the shell so the subscription outlives views.
 */
export function SurfaceEventBridge() {
  const { surfaces } = usePlatform();
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    const store = useSurfaceStore.getState();
    void surfaces
      .capabilities()
      .then(({ embedded }) => {
        if (!disposed) store.setEmbeddedSupported(embedded);
      })
      .catch(() => undefined);
    void surfaces
      .list()
      .then((records) => {
        if (!disposed) store.hydrate(records);
      })
      .catch(() => undefined);
    void surfaces
      .onEvent((event) => {
        if (!disposed) useSurfaceStore.getState().applyEvent(event);
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
  }, [surfaces]);
  return null;
}
