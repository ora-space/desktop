import type { WorkflowAgentConfig, WorkflowNodeData } from "./node-data";
import { normalizeWorkflowAgentCompletion } from "./agent-completion";

/**
 * Builds an editable/displayable projection of untrusted Agent JSON without
 * choosing an executor or authorizing malformed dependency bindings.
 *
 * `null` is a value JSON graphs can carry for an Agent node; it declares no
 * bindings, so it projects to the same empty shape as every other missing field.
 */
export function normalizeWorkflowAgentConfig(
  config: WorkflowAgentConfig | null,
): WorkflowAgentConfig {
  // Completion is removed from the spread and normalized explicitly: an edit must write the
  // declaration (and its version-4 requirement) deliberately instead of inheriting a stale shape.
  const { completion: rawCompletion, ...rest } = config ?? {};
  const completion = normalizeWorkflowAgentCompletion(rawCompletion);
  // Missing/null keeps the legacy default. Any other value is preserved verbatim so a string,
  // unsupported number, or malformed import still reaches the validator instead of being downgraded.
  const schemaVersion =
    config?.schemaVersion == null
      ? (3 as const)
      : (config.schemaVersion as WorkflowAgentConfig["schemaVersion"]);
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
  // Optional execution policies pass through untouched: absence already means the default,
  // and filling them in here would rewrite every saved graph the first time it is opened.
  return {
    ...rest,
    schemaVersion,
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
    ...(completion === undefined ? {} : { completion }),
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
    // `null` is a value JSON graphs can carry; like a missing contract it means the node
    // declares nothing, so it is left untouched instead of being projected into a config.
    if (node.data.kind !== "agent" || node.data.agentConfig == null) {
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
