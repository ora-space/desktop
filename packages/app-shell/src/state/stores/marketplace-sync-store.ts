import { create } from "zustand";

interface MarketplaceSyncState {
  /** A refresh the host started on its own, either at startup or on its recurring schedule. */
  hostRefreshing: boolean;
  /** A sync this shell started for the user, tracked here so it outlives the settings page. */
  userSyncing: boolean;
  setHostRefreshing: (running: boolean) => void;
  setUserSyncing: (running: boolean) => void;
}

/**
 * Tracks who is rebuilding the marketplace index, so the Sync action can stand down for it.
 *
 * The host admits one rebuild at a time and discards the rest, so a click made while one is
 * running would be dropped rather than served.
 *
 * The two sources are kept apart rather than folded into one flag because their spans overlap: a
 * click that slips through the race is discarded by the host and settles immediately, and a
 * shared flag would then re-enable the action while the host's own refresh was still running.
 *
 * Both live outside the settings page because a rebuild outlives it — switching panes or closing
 * the dialog mid-sync must not hand the user a button that looks ready.
 */
export const useMarketplaceSyncStore = create<MarketplaceSyncState>((set) => ({
  hostRefreshing: false,
  userSyncing: false,
  setHostRefreshing: (running) => set({ hostRefreshing: running }),
  setUserSyncing: (running) => set({ userSyncing: running }),
}));
