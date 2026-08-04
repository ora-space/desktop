import { create } from "zustand";
import type { AgentCli } from "@ora/contracts";

interface PendingAgentState {
  /** The agent chosen for one not-yet-started chat surface, keyed by its warm target. */
  selections: Record<string, AgentCli>;
  /** Records the agent chosen for one warm target, replacing any earlier pick for it. */
  setPendingAgent: (targetKey: string, agentCli: AgentCli) => void;
}

/**
 * Remembers the agent picked for a chat surface that has no session yet.
 *
 * `settings.agentCli` is the single global default offered to a target no one
 * has touched before; it cannot also hold "what this specific unstarted chat is
 * currently set to" without one target's pick leaking into another's display the
 * moment the user switches between them. This store carries that second,
 * per-target value. It is deliberately unpersisted: once a chat starts, its
 * agent lives on the session itself, so there is nothing left here worth
 * restoring after a reload.
 */
export const usePendingAgentStore = create<PendingAgentState>((set) => ({
  selections: {},
  setPendingAgent: (targetKey, agentCli) =>
    set((state) => ({ selections: { ...state.selections, [targetKey]: agentCli } })),
}));
