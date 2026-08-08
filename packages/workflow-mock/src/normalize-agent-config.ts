import type { WorkflowAgentConfig, WorkflowNodeData } from "./node-data";

/**
 * Fills MCP bindings omitted from older drafts so the settings canvas can open
 * without crashing on `agentConfig.mcps`.
 */
export function normalizeWorkflowAgentConfig(
  config: WorkflowAgentConfig,
): WorkflowAgentConfig {
  const skills = Array.isArray(config.skills) ? config.skills : [];
  const mcps = Array.isArray(config.mcps) ? config.mcps : [];
  return {
    ...config,
    skills,
    mcps,
  };
}

/** Normalizes every Agent node in a definition graph envelope. */
export function normalizeWorkflowNodeAgentConfigs<T extends {
  data: WorkflowNodeData;
}>(nodes: T[]): T[] {
  return nodes.map((node) => {
    if (node.data.kind !== "agent" || node.data.agentConfig === undefined) {
      return node;
    }
    return {
      ...node,
      data: {
        ...node.data,
        agentConfig: normalizeWorkflowAgentConfig(node.data.agentConfig),
      },
    };
  });
}
