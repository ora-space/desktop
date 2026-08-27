import { create } from "zustand";

/** A simulated round-trip drawn uniformly from `centerMs ± jitterMs`. */
export interface LatencyRange {
  centerMs: number;
  jitterMs: number;
}

/** Installing or uninstalling: 3.1415s ± 0.618s. */
export const INSTALL_LATENCY: LatencyRange = {
  centerMs: 3141.5,
  jitterMs: 618,
};

/**
 * Draws one latency from the range. `random` is injected so the spread can be
 * asserted at its bounds instead of sampled.
 */
export function simulatedLatencyMs(
  { centerMs, jitterMs }: LatencyRange,
  random: () => number = Math.random,
): number {
  return centerMs + (random() * 2 - 1) * jitterMs;
}

const sleep = (ms: number) =>
  new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });

const toggled = (ids: string[], id: string) =>
  ids.includes(id) ? ids.filter((current) => current !== id) : [...ids, id];

interface PluginInstallState {
  /** Catalog plugin ids the user has manually installed (the detection-driven CLI runtimes track their own state instead). */
  installedIds: string[];
  /** Ids with an install or uninstall still running, so their controls can show progress. */
  pendingInstallIds: string[];
  /** Installs or uninstalls a catalog plugin after a simulated round-trip. */
  toggleInstalled: (id: string) => Promise<void>;
}

/**
 * Shared install state for the hard-coded plugin catalog. Kept in one store instead of
 * component state so the Settings marketplace and the chat composer's plugin picker agree on
 * which plugins are actually available to use.
 *
 * No backend owns plugins yet, so both mutations resolve locally behind a simulated delay. The
 * state flips only once that delay elapses — the pending id lists are what every surface reads
 * to disable its control meanwhile, which also makes the operations non-reentrant.
 */
export const usePluginInstallStore = create<PluginInstallState>((set, get) => ({
  installedIds: [],
  pendingInstallIds: [],
  toggleInstalled: async (id) => {
    if (get().pendingInstallIds.includes(id)) return;
    set((state) => ({ pendingInstallIds: [...state.pendingInstallIds, id] }));
    await sleep(simulatedLatencyMs(INSTALL_LATENCY));
    set((state) => ({
      installedIds: toggled(state.installedIds, id),
      pendingInstallIds: state.pendingInstallIds.filter(
        (pending) => pending !== id,
      ),
    }));
  },
}));
