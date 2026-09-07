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
  const outputContract =
    config.outputContract?.type === "structured"
      ? {
          type: "structured" as const,
          schema: config.outputContract.schema,
        }
      : undefined;
  return {
    ...config,
    skills,
    mcps,
    // Missing `interactive` defaults to false so existing graphs stay fully automatic.
    interactive: config.interactive ?? false,
    ...(outputContract === undefined ? {} : { outputContract }),
  };
}

/** Normalizes legacy Start prompts and Agent configuration in a graph envelope. */
export function normalizeWorkflowNodeAgentConfigs<
  T extends {
    data: WorkflowNodeData;
  },
>(nodes: T[]): T[] {
  return nodes.map((node) => {
    if (
      node.data.kind === "start" &&
      node.data.input === undefined &&
      node.data.instruction !== undefined
    ) {
      const { instruction, ...data } = node.data;
      return { ...node, data: { ...data, input: instruction } };
    }
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
