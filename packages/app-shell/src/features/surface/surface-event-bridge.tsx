import { useEffect } from "react";
import { usePlatform } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";

/**
 * Hydrates the surface store from the host and streams lifecycle events into it.
 *
 * Download events are left to `SurfaceDownloadToaster`; the store only mirrors
 * instance state. Mounted once in the shell so the subscription outlives views.
 *
 * The host outlives a main-webview reload, so the snapshot may contain embedded
 * instances whose placeholder DOM is gone. They are hidden on hydrate rather than
 * adopted: the store starts with no side slot and a native view with no
 * placeholder would otherwise keep covering the fresh page.
 */
export function SurfaceEventBridge() {
  const { surfaces } = usePlatform();
  useEffect(() => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    // Instances touched by a live event before the snapshot arrives; the snapshot
    // is stale for them and must not resurrect a closed one or revert a newer state.
    const touchedBeforeHydrate = new Set<number>();
    let hydrated = false;
    const store = useSurfaceStore.getState();
    void surfaces
      .capabilities()
      .then(({ embedded }) => {
        if (!disposed) store.setEmbeddedSupported(embedded);
      })
      .catch(() => undefined);
    // Subscribe first so nothing emitted between the snapshot request and its
    // answer is lost; the snapshot is then reconciled against what already arrived.
    void surfaces
      .onEvent((event) => {
        if (disposed) return;
        if (!hydrated && "instance" in event)
          touchedBeforeHydrate.add(event.instance);
        useSurfaceStore.getState().applyEvent(event);
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined)
      .then(() => surfaces.list())
      .then((records) => {
        if (disposed) return;
        const current = useSurfaceStore.getState();
        const fresh = records.filter(
          (record) => !touchedBeforeHydrate.has(record.instance),
        );
        const kept = Object.values(current.records).filter((record) =>
          touchedBeforeHydrate.has(record.instance),
        );
        hydrated = true;
        current.hydrate([...fresh, ...kept]);
        for (const record of fresh) {
          if (record.target === "embedded") {
            void surfaces
              .setVisible(record.instance, false)
              .catch(() => undefined);
          }
        }
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [surfaces]);
  return null;
}
