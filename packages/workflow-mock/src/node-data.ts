/** Lists the node variants supported by the workflow demo. */
export const WORKFLOW_NODE_KINDS = [
  "start",
  "prompt",
  "agent",
  "condition",
  "tool",
  "output",
] as const;

export type WorkflowNodeKind = (typeof WORKFLOW_NODE_KINDS)[number];

/** Uses React Flow's `Node.data` extension point for executable workflow data. */
export interface WorkflowNodeData extends Record<string, unknown> {
  kind: WorkflowNodeKind;
  title: string;
  description: string;
  instruction: string;
  model?: string;
  tool?: string;
  condition?: string;
}
