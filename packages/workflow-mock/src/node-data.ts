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

/** Optional structured output emitted alongside an Agent node's stable text `output`. */
export interface WorkflowAgentOutputContract {
  type: "structured";
  /** JSON Schema the final assistant response must validate against. */
  schema: Record<string, unknown>;
}

/** Safe closed-object schema used when structured output is first enabled. */
export const DEFAULT_WORKFLOW_STRUCTURED_OUTPUT_SCHEMA = {
  type: "object",
  properties: {},
  required: [],
  additionalProperties: false,
} as const;

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
  /** Opt the node into a persistent interactive session that pauses for human input. */
  interactive?: boolean;
  /** Additional typed variables exposed beside the node's stable raw `output`. */
  outputContract?: WorkflowAgentOutputContract;
}

/** Value types accepted by the workflow variable pool. */
export type WorkflowVariableValueType =
  | "string"
  | "number"
  | "integer"
  | "boolean"
  | "secret"
  | "file"
  | "object"
  | "any"
  | "array"
  | "array[string]"
  | "array[number]"
  | "array[object]"
  | "array[boolean]"
  | "array[file]"
  | "array[any]";

/** Durable reference to one file below the workflow run's Workspace root. */
export interface WorkflowFileReference {
  kind: "workspace_file";
  /** Slash-normalized relative path; absolute and parent-traversing paths are invalid. */
  path: string;
}

/** One workflow-wide variable available independently of graph topology. */
export interface WorkflowGlobalVariable {
  name: string;
  valueType: WorkflowVariableValueType;
  value?: unknown;
}

/** Form controls supported by a Start node when collecting deployment-time input. */
export type WorkflowInputFieldType =
  | "text-input"
  | "paragraph"
  | "select"
  | "number"
  | "checkbox"
  | "file"
  | "file-list"
  | "json";

/** One typed input declared by the Start node. */
export interface WorkflowInputVariable {
  name: string;
  /** Human-facing label shown when a deployed run collects this value. */
  displayName?: string;
  /** Presentation control; kept separate because several controls produce the same value type. */
  fieldType?: WorkflowInputFieldType;
  valueType: WorkflowVariableValueType;
  /** Whether a deployed run must provide a non-empty value before execution. */
  required?: boolean;
  /** Choices presented by a select field. */
  options?: string[];
  /** Character limit for string-like values. */
  maxLength?: number;
  /** Optional initial value. Missing means the deployment must provide it before execution. */
  value?: unknown;
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

/**
 * One comparison in the executable Condition format: a fully-qualified variable selector, a
 * typed operator, and an optional expected value. Mirrors the backend's `data.cases` wire shape.
 */
export interface WorkflowConditionComparison {
  /** Dify-style selector `["nodeId", "root", ...nestedPath]`. */
  variableSelector: string[];
  operator: string;
  /** Expected value, omitted for operators such as `exists` / `empty`. */
  value?: unknown;
}

/** Operators whose meaning does not require a literal comparison value. */
const VALUELESS_CONDITION_OPERATORS = new Set([
  "empty",
  "not_empty",
  "exists",
  "not_exists",
]);

/** Whether a condition row has enough authored data to be presented as configured. */
export function isWorkflowConditionComparisonComplete(
  comparison: WorkflowConditionComparison,
): boolean {
  if (
    comparison.variableSelector.length < 2 ||
    comparison.variableSelector.some((part) => part.trim() === "") ||
    comparison.operator.trim() === ""
  ) {
    return false;
  }
  if (VALUELESS_CONDITION_OPERATORS.has(comparison.operator)) {
    return true;
  }
  return (
    comparison.value !== undefined &&
    comparison.value !== null &&
    (typeof comparison.value !== "string" || comparison.value.trim() !== "")
  );
}

/** One executable case of a Condition node; the trailing "else" path is implicit. */
export interface WorkflowConditionCase {
  id: string;
  logic?: WorkflowConditionLogic;
  conditions: WorkflowConditionComparison[];
}

/** Returns canonical cases while keeping existing saved workflow graphs readable. */
export function resolveConditionCases(
  data: WorkflowNodeData,
): WorkflowConditionCase[] {
  if (data.cases !== undefined) {
    return data.cases;
  }
  if (data.conditionCases !== undefined) {
    return data.conditionCases;
  }
  const branches = data.conditionBranches;
  if (branches !== undefined && branches.length > 0) {
    return branches.map((branch, index) => ({
      id: `case-${index + 1}`,
      logic: branch.logic ?? "and",
      conditions: branch.conditions.map((rule) => ({
        variableSelector: rule.variable
          .split(".")
          .map((part) => part.trim())
          .filter((part) => part !== ""),
        operator: migrateConditionOperator(rule.operator, rule.negated),
        ...(rule.value !== "" ? { value: rule.value } : {}),
      })),
    }));
  }
  return [
    {
      id: "case-1",
      logic: "and",
      conditions: [],
    },
  ];
}

/** Normalizes legacy operators before the graph is saved in the canonical `cases` field. */
function migrateConditionOperator(
  operator: string,
  negated: boolean | undefined,
): string {
  const normalized =
    operator === "is_empty"
      ? "empty"
      : operator === "is_not_empty"
        ? "not_empty"
        : operator;
  if (negated !== true) {
    return normalized;
  }
  const negatedForms: Record<string, string> = {
    equals: "not_equals",
    contains: "not_contains",
    empty: "not_empty",
  };
  return negatedForms[normalized] ?? normalized;
}

/** One named result an Output node exposes, resolved from the run variable pool at completion. */
export interface WorkflowOutputBinding {
  name: string;
  /** Dify-style selector `["nodeId", "root", ...nestedPath]`. */
  variableSelector: string[];
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
  /** Start node: initial prompt used as the run's kickoff input. */
  input?: string;
  /** Human node: prompt shown to the reviewer. */
  instruction?: string;
  /** Legacy scheduling metadata; no current runtime consumes this value. */
  trigger?: string;
  /** Start node: variables the workflow receives on start. */
  inputVariables?: WorkflowInputVariable[];
  tool?: string;
  condition?: string;
  agentConfig?: WorkflowAgentConfig;
  conditionBranches?: WorkflowConditionBranch[];
  /** Legacy executable cases retained so existing saved graphs can be migrated on edit. */
  conditionCases?: WorkflowConditionCase[];
  /** Executable cases for Condition nodes, matching the backend `data.cases` wire format. */
  cases?: WorkflowConditionCase[];
  /** Named result bindings of an Output node, resolved from the variable pool at completion. */
  outputs?: WorkflowOutputBinding[];
  operation?: string;
  toolParameters?: WorkflowToolParameter[];
  waitStrategy?: WorkflowJunctionWaitStrategy;
  waitCount?: number;
  failureStrategy?: WorkflowJunctionFailureStrategy;
  maxAttempts?: number;
  exitCondition?: string;
  /**
   * Fixture-only mock-engine step duration (ms); the editor deliberately does
   * not expose this simulation control as workflow configuration.
   */
  mockStepMs?: number;
}
