import type {
  WorkflowAgentConfig,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import { AGENT_CLI_LABELS, type KnownAgentCli } from "../chat/model-catalog";

/** True when the executor CLI id is one of the known product Agent CLIs. */
export function isKnownAgentCli(agentCli: string): agentCli is KnownAgentCli {
  return agentCli in AGENT_CLI_LABELS;
}

/** Formats `CLI · modelId` with a human CLI label when the CLI is known. */
export function formatAgentExecutorLabel(
  executor: WorkflowAgentConfig["executor"],
): string {
  const cliLabel = isKnownAgentCli(executor.agentCli)
    ? AGENT_CLI_LABELS[executor.agentCli]
    : executor.agentCli;
  return `${cliLabel} · ${executor.modelId}`;
}

/**
 * Theater mono detail line: flat tool/condition first, else agent executor.
 * Keeps the stage glance to one quiet line.
 */
export function resolveTheaterActDetail(
  data: WorkflowNodeData,
): string | undefined {
  for (const candidate of [data.tool, data.condition]) {
    const trimmed = candidate?.trim();
    if (trimmed !== undefined && trimmed !== "") {
      return trimmed;
    }
  }
  if (data.agentConfig !== undefined) {
    return formatAgentExecutorLabel(data.agentConfig.executor);
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
