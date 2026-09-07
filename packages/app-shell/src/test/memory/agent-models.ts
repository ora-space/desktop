import type * as acp from "@agentclientprotocol/sdk";

/** Model-discovery fixture shared by sessions and agent-runtime adapters. */
export interface AgentModelMemoryState {
  /** What every started session reports as its configuration. */
  configOptions: acp.SessionConfigOption[];
  /**
   * Per-CLI model discovery overrides for tests. A CLI mapped to `null`
   * reports no model catalog; a CLI mapped to an array uses
   * those options instead of the shared `configOptions`.
   */
  agentModelsByCli?: Partial<Record<string, acp.SessionConfigOption[] | null>>;
}

/** Creates the explicit two-model catalog used by composer fixtures. */
export function createAgentModelMemory(): AgentModelMemoryState {
  return {
    configOptions: [
      {
        id: "model",
        name: "Model",
        category: "model",
        type: "select",
        currentValue: "opencode/big-pickle",
        options: [
          { value: "opencode/big-pickle", name: "Big Pickle" },
          { value: "opencode/small-pickle", name: "Small Pickle" },
        ],
      },
    ],
  };
}
