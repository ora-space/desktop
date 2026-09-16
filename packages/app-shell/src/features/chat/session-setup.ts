import { create } from "zustand";

/** Frontend-only timing for provider setup before a prompt. */
export type SessionSetupPresentation =
  | {
      id: string;
      turnIndex: number;
      status: "connecting";
      startedAt: number;
    }
  | {
      id: string;
      turnIndex: number;
      status: "connected" | "failed";
      startedAt: number;
      durationMs: number;
    };

interface SessionSetupState {
  setups: Record<string, SessionSetupPresentation[]>;
  /** Adds or settles one setup phase in its session-scoped runtime cache. */
  upsert: (sessionId: string, setup: SessionSetupPresentation) => void;
}

/**
 * Retains setup chrome across chat navigation and view remounts, but never persists it.
 * Restarting Ora creates a fresh renderer store, so history cannot manufacture old timings.
 */
export const useSessionSetupStore = create<SessionSetupState>((set) => ({
  setups: {},
  upsert: (sessionId, setup) =>
    set((state) => {
      const existing = state.setups[sessionId] ?? [];
      return {
        setups: {
          ...state.setups,
          [sessionId]: [
            ...existing.filter((item) => item.id !== setup.id),
            setup,
          ],
        },
      };
    }),
}));
