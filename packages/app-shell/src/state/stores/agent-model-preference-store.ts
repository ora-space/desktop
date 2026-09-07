import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";

export const AGENT_MODEL_PREFERENCE_STORAGE_KEY =
  "ora.agent-model-preference.v1";

interface AgentModelPreferenceState {
  /** The model last chosen for each agent, keyed by that agent's namespaced ref. */
  models: Record<string, string>;
  /** Records the model a user picked for an agent, replacing any earlier one. */
  rememberModel: (agentRef: string, model: string) => void;
}

/**
 * Remembers, per agent, which model the user last started a chat on.
 *
 * Scope is deliberately narrow: this answers "what should a chat that has not
 * started yet open on", nothing else. A persisted session carries its own model
 * in the configuration its agent reports, so an ongoing conversation never reads
 * or writes here and cannot be moved by a pick made on some other surface.
 *
 * Keyed by agent rather than by chat surface, which is what makes it a
 * preference instead of an intent: `usePendingAgentStore` already holds "what is
 * this one unstarted surface set to" and is discarded once that chat starts.
 * This outlives the send, so the next new chat on the same agent opens where the
 * last one did — and switching agents shows each agent's own last model instead
 * of a value that only made sense for the other one.
 *
 * Persisted to localStorage only: which model a machine's user prefers is a
 * client convenience the backend has no concept of. Stored values are open
 * strings and are never validated here — catalogs change with plugin versions,
 * so the picker checks a remembered model against the live catalog before
 * offering it.
 */
export const useAgentModelPreferenceStore = create<AgentModelPreferenceState>()(
  persist(
    (set) => ({
      models: {},
      rememberModel: (agentRef, model) =>
        set((state) => ({ models: { ...state.models, [agentRef]: model } })),
    }),
    {
      name: AGENT_MODEL_PREFERENCE_STORAGE_KEY,
      storage: createJSONStorage(() => window.localStorage),
      // Tolerate partial or corrupt persisted state: a missing map is the same
      // situation as a first run, and every consumer already handles an agent
      // with no remembered model.
      merge: (persisted, current) => {
        const persistedModels = (
          persisted as Partial<AgentModelPreferenceState> | undefined
        )?.models;
        return {
          ...current,
          models:
            persistedModels !== null && typeof persistedModels === "object"
              ? persistedModels
              : {},
        };
      },
    },
  ),
);
