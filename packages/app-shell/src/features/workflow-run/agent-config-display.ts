import {
  resolveWorkflowAgentRetryPolicy,
  workflowAgentRetryApplies,
} from "@ora/workflow-mock";
import type {
  WorkflowAgentConfig,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import {
  agentLabel,
  type AgentEntry,
} from "../../state/hooks/use-agent-catalog";

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
 * Theater instruction body: Start input, then the flat node instruction, else Agent prompt.
 *
 * Start nodes read their kickoff text from `input`; every other node prefers the flat
 * `instruction` (Human prompt, or a legacy Agent instruction retained on older saved graphs)
 * and falls back to `agentConfig.prompt` for Agent nodes authored under the current model.
 * Empty / whitespace-only values collapse so the stage can show an em dash.
 */
export function resolveTheaterActInstruction(data: WorkflowNodeData): string {
  const text =
    data.kind === "start"
      ? (data.input ?? "")
      : (data.instruction ?? data.agentConfig?.prompt ?? "");
  return text.trim();
}

/**
 * What the run inspector reports about a node's automatic retry: interactive nodes are never
 * retried, a turned-off policy is shown as such, an enabled policy with zero retries never
 * reruns either, and any other enabled one says whether it is the default (field absent in the
 * snapshot) or the author's own values.
 */
export type AgentRetryDisplay =
  | { kind: "interactive" }
  | { kind: "disabled" }
  | { kind: "noRetries" }
  | {
      kind: "enabled";
      source: "default" | "configured";
      maxRetries: number;
      initialDelaySeconds: number;
    };

/** Resolves the retry policy the engine applies to this node's snapshot configuration. */
export function resolveAgentRetryDisplay(
  config: WorkflowAgentConfig,
): AgentRetryDisplay {
  if (!workflowAgentRetryApplies(config)) {
    return { kind: "interactive" };
  }
  const policy = resolveWorkflowAgentRetryPolicy(config);
  if (!policy.enabled) {
    return { kind: "disabled" };
  }
  if (policy.maxRetries === 0) {
    return { kind: "noRetries" };
  }
  return {
    kind: "enabled",
    source: config.retry == null ? "default" : "configured",
    maxRetries: policy.maxRetries,
    initialDelaySeconds: policy.initialDelaySeconds,
  };
}
