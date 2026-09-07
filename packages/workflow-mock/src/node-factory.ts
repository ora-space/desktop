import type { Node, XYPosition } from "@xyflow/react";
import {
  createMockWorkflowCapabilities,
  createMockWorkflowNodeType,
} from "./capabilities";
import type {
  WorkflowAgentConfig,
  WorkflowNodeData,
  WorkflowNodeKind,
} from "./node-data";

/** Creates a catalog item as a native React Flow node with business data in `data`. */
export function createMockWorkflowNode({
  kind,
  sequence,
  position,
  locale,
  agentConfig,
}: {
  kind: WorkflowNodeKind;
  sequence: number;
  position: XYPosition;
  locale: "zh-CN" | "en-US";
  agentConfig?: WorkflowAgentConfig;
}): Node<WorkflowNodeData, "workflow"> {
  const nodeType = createMockWorkflowNodeType(kind, locale);
  return {
    id: `${kind}-${sequence}`,
    type: "workflow",
    ...(kind === "start" ? { deletable: false } : {}),
    position: { ...position },
    data: {
      kind,
      title: `${nodeType.label} ${sequence}`,
      description: nodeType.description,
      ...createMockNodeExecutionData(kind, locale, agentConfig),
    },
  };
}

/** Provides deterministic values for React Flow's node-data execution extension. */
function createMockNodeExecutionData(
  kind: WorkflowNodeKind,
  locale: "zh-CN" | "en-US",
  agentConfig: WorkflowAgentConfig | undefined,
): Pick<
  WorkflowNodeData,
  | "agentConfig"
  | "input"
  | "instruction"
  | "tool"
  | "condition"
  | "cases"
  | "waitStrategy"
  | "failureStrategy"
  | "maxAttempts"
  | "exitCondition"
> {
  const capabilities = createMockWorkflowCapabilities(locale);
  switch (kind) {
    case "start":
      return { input: "" };
    case "output":
      return {};
    case "human":
    case "subflow":
      return {};
    case "agent":
      return {
        agentConfig: structuredClone(
          agentConfig ?? capabilities.defaultAgentConfig,
        ),
      };
    case "condition":
      return {
        condition: locale === "zh-CN" ? "满足条件" : "Condition is met",
        // Keeping the first IF branch explicit makes its output handle stable before rules exist.
        cases: [{ id: "case-1", logic: "and", conditions: [] }],
      };
    case "tool":
      return { tool: capabilities.defaultTool };
    case "junction":
      return { waitStrategy: "all", failureStrategy: "fail" };
    case "loop":
      return { maxAttempts: 3, exitCondition: "" };
  }
}
