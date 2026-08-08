/** Lists the node variants supported by the workflow demo. */
export const WORKFLOW_NODE_KINDS = [
  "start",
  "agent",
  "condition",
  "tool",
  "junction",
  "human",
  "loop",
  "subflow",
  "output",
] as const;

export type WorkflowNodeKind = (typeof WORKFLOW_NODE_KINDS)[number];

/** Stores one configured Agent Skill and whether it is available during execution. */
export interface WorkflowAgentSkillConfig {
  skillId: string;
  enabled: boolean;
}

/** Stores one configured MCP binding and whether it is available during execution. */
export interface WorkflowAgentMcpConfig {
  mcpId: string;
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
  /** Optional MCP attachments; empty means the node uses no MCP servers. */
  mcps: WorkflowAgentMcpConfig[];
  prompt: string;
}

/** One named input variable exposed to a Prompt node's template. */
export interface WorkflowInputVariable {
  name: string;
  /** Default value, usually referencing a context variable like `{{repository}}`. */
  defaultValue?: string;
}

/** One rule inside a condition branch: a variable, a comparison operator, and an expected value. */
export interface WorkflowConditionRule {
  variable: string;
  operator: string;
  value: string;
  /** When true, the rule is negated (NOT). */
  negated?: boolean;
}

/** How the rules inside a branch combine: all of them (AND) or any of them (OR). */
export type WorkflowConditionLogic = "and" | "or";

/** One IF branch of a Condition node; the trailing "otherwise" path is implicit. */
export interface WorkflowConditionBranch {
  conditions: WorkflowConditionRule[];
  logic?: WorkflowConditionLogic;
}

/** Which branches a Junction node waits for before it may proceed. */
export type WorkflowJunctionWaitStrategy = "all" | "any" | "count";

/** How a Junction node reacts when one of its upstream branches fails. */
export type WorkflowJunctionFailureStrategy = "fail" | "continue";

/** One key/value call parameter passed to the selected Tool node. */
export interface WorkflowToolParameter {
  key: string;
  value: string;
}

/** Uses React Flow's `Node.data` extension point for executable workflow data. */
export interface WorkflowNodeData extends Record<string, unknown> {
  kind: WorkflowNodeKind;
  title: string;
  description: string;
  instruction?: string;
  /** Start node: how the workflow is triggered (merge request, push, manual). */
  trigger?: string;
  /** Start node: variables the workflow receives on start. */
  inputVariables?: WorkflowInputVariable[];
  tool?: string;
  condition?: string;
  agentConfig?: WorkflowAgentConfig;
  /** Structured IF/ELSE rules for Condition nodes (replaces the flat condition string). */
  conditionBranches?: WorkflowConditionBranch[];
  /** Selected operation of the Tool node, resolved from the tool's operation catalog. */
  operation?: string;
  /** Key/value call parameters for the Tool node. */
  toolParameters?: WorkflowToolParameter[];
  /** Junction node: which upstream branches must finish before it proceeds. */
  waitStrategy?: WorkflowJunctionWaitStrategy;
  /** Junction node: minimum branch count when the wait strategy is "count". */
  waitCount?: number;
  /** Junction node: behavior when an upstream branch fails. */
  failureStrategy?: WorkflowJunctionFailureStrategy;
  /** Loop node: maximum iterations before the loop gives up. */
  maxAttempts?: number;
  /** Loop node: condition that ends the loop early, shown as a readable rule. */
  exitCondition?: string;
  /**
   * Optional mock-engine step duration (ms). When set, that node runs for this
   * long instead of the runtime default — used for staggered parallel demos.
   */
  mockStepMs?: number;
}
