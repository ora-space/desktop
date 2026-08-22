import { useSyncExternalStore } from "react";

/**
 * Subscribes to a zustand persist store's hydration so callers can wait for
 * disk state before applying first-run defaults that would overwrite it.
 */
export function usePersistHydrated(persistApi: {
  hasHydrated: () => boolean;
  onFinishHydration: (fn: (state: unknown) => void) => () => void;
}): boolean {
  return useSyncExternalStore(
    (onStoreChange) => persistApi.onFinishHydration(onStoreChange),
    () => persistApi.hasHydrated(),
    () => false,
  );
}
