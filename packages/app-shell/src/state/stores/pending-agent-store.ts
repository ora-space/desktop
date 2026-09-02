import { create } from "zustand";

interface PendingAgentState {
  /** The agent chosen for one not-yet-started chat surface, keyed by its target. */
  selections: Record<string, string>;
  /** The agent a persisted session is set to move onto, keyed by that session's id. */
  switches: Record<string, string>;
  /** Model intent for a surface and agent before that agent's provider session exists. */
  models: Record<string, string>;
  /** Records the agent chosen for one target, replacing any earlier pick for it. */
  setPendingAgent: (targetKey: string, agentCli: string) => void;
  /** Records that a session should be rebound onto `agentCli` when it is next sent into. */
  setPendingSwitch: (sessionId: string, agentCli: string) => void;
  /** Drops a session's recorded move once it has been committed or abandoned. */
  clearPendingSwitch: (sessionId: string) => void;
  /** Records the model sent into start or switch for this exact surface and agent. */
  setPendingModel: (key: string, model: string) => void;
  /** Drops a model intent after it has been applied to a real provider session. */
  clearPendingModel: (key: string) => void;
}

/**
 * Remembers the agent a chat surface is set to use before anything commits it.
 *
 * Two shapes of the same idea. For a chat with no session yet, `settings.agentCli`
 * is the single global default offered to a target no one has touched before; it
 * cannot also hold "what this specific unstarted chat is currently set to"
 * without one target's pick leaking into another's display the moment the user
 * switches between them, so `selections` carries that per-target value.
 *
 * For a persisted session, `switches` holds a move the user has chosen but not
 * yet paid for. The rebind itself waits for the next message, because performing
 * it at click time would tear down the agent that is mid-reply. Until then this
 * is the only record that the picker — and the first send that creates its session —
 * belong to the incoming CLI rather than the bound one.
 *
 * Deliberately unpersisted: once a chat starts or a move commits, the agent
 * lives on the session itself, so nothing here is worth restoring after a reload.
 */
export const usePendingAgentStore = create<PendingAgentState>((set) => ({
  selections: {},
  switches: {},
  models: {},
  setPendingAgent: (targetKey, agentCli) =>
    set((state) => ({
      selections: { ...state.selections, [targetKey]: agentCli },
    })),
  setPendingSwitch: (sessionId, agentCli) =>
    set((state) => ({
      switches: { ...state.switches, [sessionId]: agentCli },
    })),
  clearPendingSwitch: (sessionId) =>
    set((state) => {
      const switches = { ...state.switches };
      delete switches[sessionId];
      return { switches };
    }),
  setPendingModel: (key, model) =>
    set((state) => ({ models: { ...state.models, [key]: model } })),
  clearPendingModel: (key) =>
    set((state) => {
      const models = { ...state.models };
      delete models[key];
      return { models };
    }),
}));

/** Names a pre-session model intent without conflating two agents on one surface. */
export function pendingModelKey(
  selection: {
    projectId: string | null;
    taskId: string | null;
    sessionId: string | null;
  },
  agentRef: string,
): string {
  const surface =
    selection.sessionId !== null
      ? `session:${selection.sessionId}`
      : selection.taskId !== null
        ? `task:${selection.taskId}`
        : `project:${selection.projectId ?? "none"}`;
  return `${surface}|agent:${agentRef}`;
}

/**
 * Reports the agent a persisted session is set to move onto, if any.
 *
 * Distinct from the CLI a surface resolves to: callers need to tell "this
 * surface is showing agent B" apart from "this surface is showing agent B
 * *because a move is pending*". Only the latter creates a session for the incoming
 * CLI and commits the rebind on the next send.
 */
export function usePendingSwitch(sessionId: string | null): string | undefined {
  return usePendingAgentStore((state) =>
    sessionId === null ? undefined : state.switches[sessionId],
  );
}
