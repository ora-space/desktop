import type { WorkflowAgentConfig, WorkflowNodeData } from "./node-data";

/**
 * Builds an editable/displayable projection of untrusted Agent JSON without
 * choosing an executor or authorizing malformed dependency bindings.
 */
export function normalizeWorkflowAgentConfig(
  config: WorkflowAgentConfig,
): WorkflowAgentConfig {
  const skills = Array.isArray(config?.skills)
    ? config.skills.filter(
        (skill) =>
          typeof skill?.skillId === "string" &&
          typeof skill.enabled === "boolean",
      )
    : [];
  const mcps = Array.isArray(config?.mcps)
    ? config.mcps.filter(
        (mcp) =>
          typeof mcp?.mcpId === "string" && typeof mcp.enabled === "boolean",
      )
    : [];
  const outputContract =
    config?.outputContract?.type === "structured" &&
    typeof config.outputContract.schema === "object" &&
    config.outputContract.schema !== null &&
    !Array.isArray(config.outputContract.schema)
      ? {
          type: "structured" as const,
          schema: config.outputContract.schema,
        }
      : undefined;
  // `retry` passes through untouched: absent already means the default policy, and filling it in
  // here would rewrite every saved graph the first time it is opened.
  return {
    ...config,
    executor: {
      ...config?.executor,
      agentCli:
        typeof config?.executor?.agentCli === "string"
          ? config.executor.agentCli
          : "",
      modelId:
        typeof config?.executor?.modelId === "string"
          ? config.executor.modelId
          : "",
    },
    roleId: typeof config?.roleId === "string" ? config.roleId : "",
    prompt: typeof config?.prompt === "string" ? config.prompt : "",
    skills,
    mcps,
    // Missing `interactive` defaults to false so existing graphs stay fully automatic.
    interactive: config?.interactive === true,
    outputContract,
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
