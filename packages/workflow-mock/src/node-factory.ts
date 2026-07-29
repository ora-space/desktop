import type {
  WorkflowLocale,
  WorkflowNode,
  WorkflowNodeConfig,
  WorkflowNodeKind,
  WorkflowPosition,
} from "./types";
import {
  createMockWorkflowCapabilities,
  createMockWorkflowNodeType,
} from "./capabilities";

export interface CreateMockWorkflowNodeOptions {
  kind: WorkflowNodeKind;
  sequence: number;
  position: WorkflowPosition;
  locale: WorkflowLocale;
}

/** Creates a catalog node while keeping prototype-only configuration defaults out of the UI. */
export function createMockWorkflowNode({
  kind,
  sequence,
  position,
  locale,
}: CreateMockWorkflowNodeOptions): WorkflowNode {
  const nodeType = createMockWorkflowNodeType(kind, locale);
  return {
    id: `${kind}-${sequence}`,
    kind,
    title: `${nodeType.label} ${sequence}`,
    description: nodeType.description,
    position: { ...position },
    config: createMockNodeConfig(kind, locale),
  };
}

/** Provides deterministic mock configuration for every supported node kind. */
function createMockNodeConfig(
  kind: WorkflowNodeKind,
  locale: WorkflowLocale,
): WorkflowNodeConfig {
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
