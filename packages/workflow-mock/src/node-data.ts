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

/** Stores one configured Agent Skill and whether it is available during execution. */
export interface WorkflowAgentSkillConfig {
  skillId: string;
  enabled: boolean;
}

/** Stores the execution contract for an Agent node without relying on display labels. */
export interface WorkflowAgentConfig {
  schemaVersion: 3;
  executor: {
    agentCli: string;
    modelId: string;
  };
  roleId: string;
  skills: WorkflowAgentSkillConfig[];
  prompt: string;
}

/** Uses React Flow's `Node.data` extension point for executable workflow data. */
export interface WorkflowNodeData extends Record<string, unknown> {
  kind: WorkflowNodeKind;
  title: string;
  description: string;
  instruction?: string;
  model?: string;
  tool?: string;
  condition?: string;
  agentConfig?: WorkflowAgentConfig;
  /**
   * Optional mock-engine step duration (ms). When set, that node runs for this
   * long instead of the runtime default — used for staggered parallel demos.
   */
  mockStepMs?: number;
}
