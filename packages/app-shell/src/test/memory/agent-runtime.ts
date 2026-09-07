import { type AgentRuntimeStatus, type InstalledPlugin } from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import {
  createAgentModelMemory,
  type AgentModelMemoryState,
} from "./agent-models";
import { seededAgentPackages } from "./agent-packages";

/** Mutable records owned by the agent-runtime test adapter. */
export interface AgentRuntimeMemoryState extends AgentModelMemoryState {
  /**
   * What the agent runtime reports reaching, which is what decides the agents the pickers offer.
   *
   * An agent missing from this list is one nothing supervises — an uninstalled plugin package.
   */
  agentRuntimeStatuses: AgentRuntimeStatus[];
}

/** Creates an independent agent-runtime memory fixture. */
export function createAgentRuntimeMemory(
  installedPlugins: readonly InstalledPlugin[] = seededAgentPackages(),
): AgentRuntimeMemoryState {
  return {
    agentRuntimeStatuses: installedPlugins.map((plugin) => ({
      agentRef: plugin.id,
      status: "ready",
    })),
    ...createAgentModelMemory(),
  };
}

/** Registers only the agent-runtime operations explicitly requested by a fixture. */
export function agentRuntimeHandlers(state: AgentRuntimeMemoryState) {
  return {
    getAgentRuntimeStatus: async () => ({
      statuses: [...state.agentRuntimeStatuses],
    }),
    listAgentModels: async (req) => {
      const options = state.agentModelsByCli?.[req.agentRef];
      const configOptions =
        options === undefined ? state.configOptions : options;
      if (configOptions === null) return { models: [] };
      const selector = configOptions.find(
        (option) => option.type === "select" && option.category === "model",
      );
      if (selector?.type !== "select") return { models: [] };
      return {
        models: selector.options.flatMap((entry) =>
          "group" in entry
            ? entry.options.map((option) => ({
                id: option.value,
                displayName: option.name,
                default: option.value === selector.currentValue,
              }))
            : [
                {
                  id: entry.value,
                  displayName: entry.name,
                  default: entry.value === selector.currentValue,
                },
              ],
        ),
      };
    },
  } satisfies TestHandlers;
}
