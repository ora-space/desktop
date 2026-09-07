import { type Agent } from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { nextId } from "./records";

/** Mutable records owned by the agents test adapter. */
export interface AgentMemoryState {
  agents: Agent[];
}

/** Creates an independent agents memory fixture. */
export function createAgentMemory(): AgentMemoryState {
  return { agents: [] };
}

/** Registers only the agents operations explicitly requested by a fixture. */
export function agentHandlers(state: AgentMemoryState) {
  return {
    listAgents: async () => ({ agents: [...state.agents] }),
    getAgent: async (req) => ({
      agent: {
        ...state.agents.find((a) => a.id === req.agentId)!,
        content: "",
      },
    }),
    createAgent: async (req) => {
      const agent: Agent = {
        id: nextId("a", state.agents.length),
        namespace: "local",
        name: req.name,
        description: req.description,
      };
      state.agents.push(agent);
      return { agent };
    },
    updateAgent: async (req) => {
      const idx = state.agents.findIndex((a) => a.id === req.agentId);
      if (idx < 0) throw new Error(`agent ${req.agentId} not found`);
      const updated: Agent = {
        id: req.agentId,
        namespace: state.agents[idx].namespace,
        name: req.name,
        description: req.description,
      };
      state.agents[idx] = updated;
      return { agent: updated };
    },
    deleteAgent: async (req) => {
      const idx = state.agents.findIndex((a) => a.id === req.agentId);
      if (idx >= 0) state.agents.splice(idx, 1);
      return { agentId: req.agentId };
    },
  } satisfies TestHandlers;
}
