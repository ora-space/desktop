import type {
  WorkflowAgentConfig,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import { agentLabel, type AgentEntry } from "../chat/agent-catalog";

/**
 * Formats `agent · modelId`, naming the agent the way its package does.
 *
 * A run can outlive the package that supplied its agent, so an identity the catalog no longer
 * carries falls back to the identity itself: that is what the run was actually executed on.
 */
export function formatAgentExecutorLabel(
  executor: WorkflowAgentConfig["executor"],
  agents: readonly AgentEntry[],
): string {
  return `${agentLabel(agents, executor.agentCli)} · ${executor.modelId}`;
}

/**
 * Theater mono detail line: flat tool/condition first, else agent executor.
 * Keeps the stage glance to one quiet line.
 */
export function resolveTheaterActDetail(
  data: WorkflowNodeData,
  agents: readonly AgentEntry[],
): string | undefined {
  for (const candidate of [data.tool, data.condition]) {
    const trimmed = candidate?.trim();
    if (trimmed !== undefined && trimmed !== "") {
      return trimmed;
    }
  }
  if (data.agentConfig !== undefined) {
    return formatAgentExecutorLabel(data.agentConfig.executor, agents);
  }
  return undefined;
}

/**
 * Theater instruction body: flat instruction, else agent prompt.
 * Empty / whitespace-only values collapse so the stage can show an em dash.
 */
export function resolveTheaterActInstruction(data: WorkflowNodeData): string {
  const text = data.instruction ?? data.agentConfig?.prompt ?? "";
  return text.trim();
}
