import { create } from "zustand";

interface MarketplaceSyncState {
  /** A sync this shell started for the user, tracked here so it outlives the settings page. */
  userSyncing: boolean;
  setUserSyncing: (running: boolean) => void;
}

/**
 * Tracks the marketplace sync this shell started for the user.
 *
 * It lives outside the settings page because the rebuild behind it outlives that page: leaving
 * the plugin settings mid-sync and reopening them would otherwise restore a Sync button that
 * looks ready while the backend is still rebuilding the index.
 */
export const useMarketplaceSyncStore = create<MarketplaceSyncState>((set) => ({
  userSyncing: false,
  setUserSyncing: (running) => set({ userSyncing: running }),
}));
