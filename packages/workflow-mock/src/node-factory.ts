import type { Node, XYPosition } from "@xyflow/react";
import {
  createMockWorkflowCapabilities,
  createMockWorkflowNodeType,
} from "./capabilities";
import type { WorkflowNodeData, WorkflowNodeKind } from "./node-data";

/** Creates a catalog item as a native React Flow node with business data in `data`. */
export function createMockWorkflowNode({
  kind,
  sequence,
  position,
  locale,
}: {
  kind: WorkflowNodeKind;
  sequence: number;
  position: XYPosition;
  locale: "zh-CN" | "en-US";
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
      ...createMockNodeExecutionData(kind, locale),
    },
  };
}

/** Provides deterministic values for React Flow's node-data execution extension. */
function createMockNodeExecutionData(
  kind: WorkflowNodeKind,
  locale: "zh-CN" | "en-US",
): Pick<WorkflowNodeData, "instruction" | "model" | "tool" | "condition"> {
  const capabilities = createMockWorkflowCapabilities(locale);
  switch (kind) {
    case "start":
    case "output":
      return { instruction: "" };
    case "prompt":
    case "agent":
      return { instruction: "", model: capabilities.defaultModel };
    case "condition":
      return {
        instruction: "",
        condition: locale === "zh-CN" ? "满足条件" : "Condition is met",
      };
    case "tool":
      return { instruction: "", tool: capabilities.defaultTool };
  }
}
