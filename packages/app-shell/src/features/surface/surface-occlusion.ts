import { useSyncExternalStore } from "react";
import { useOcclusionLeaseCount } from "@ora/ui";
import { useSurfaceStore } from "../../state/stores/surface-store";

export { useOcclusionLease } from "@ora/ui";

function subscribeDocumentVisibility(listener: () => void): () => void {
  document.addEventListener("visibilitychange", listener);
  return () => document.removeEventListener("visibilitychange", listener);
}

function documentVisible(): boolean {
  return document.visibilityState === "visible";
}

/**
 * Whether the embedded surface for `instance` may be shown right now.
 *
 * A native child view paints above every DOM overlay, so it must hide while any
 * dialog or menu holds an occlusion lease, while the document is hidden (which
 * also covers a minimised window), and whenever another instance owns the slot.
 * Window blur deliberately does not hide it.
 */
export function useSurfaceVisibility(instance: number): boolean {
  const leaseCount = useOcclusionLeaseCount();
  const visibleDocument = useSyncExternalStore(
    subscribeDocumentVisibility,
    documentVisible,
    () => true,
  );
  const sidePanelInstance = useSurfaceStore((s) => s.sidePanelInstance);
  return leaseCount === 0 && visibleDocument && sidePanelInstance === instance;
}

/** True when an embedded surface occupies the side slot and is currently shown. */
export function useEmbeddedSurfaceVisible(): boolean {
  const sidePanelInstance = useSurfaceStore((s) => s.sidePanelInstance);
  // -1 never matches a host instance id, so the hook reports false without a slot owner.
  return useSurfaceVisibility(sidePanelInstance ?? -1);
}
