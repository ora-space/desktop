/** Node variants understood by the graph workflow execution contract. */
export type WorkflowNodeKind =
  | "start"
  | "agent"
  | "condition"
  | "tool"
  | "junction"
  | "human"
  | "loop"
  | "subflow"
  | "output";

/** One Skill binding within an executable Agent node. */
export interface WorkflowAgentSkillConfig {
  skillId: string;
  enabled: boolean;
}

/** One MCP binding within an executable Agent node. */
export interface WorkflowAgentMcpConfig {
  mcpId: string;
  enabled: boolean;
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

/** One executable case of a Condition node; the trailing "else" path is implicit. */
export interface WorkflowConditionCase {
  id: string;
  logic?: WorkflowConditionLogic;
  conditions: WorkflowConditionComparison[];
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

/** Optional structured output emitted alongside an Agent node's stable text `output`. */
export interface WorkflowAgentOutputContract {
  type: "structured";
  /** JSON Schema the final assistant response must validate against. */
  schema: Record<string, unknown>;
}

/** Transport-neutral execution contract for an Agent node. */
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
  /**
   * Whether the node's final assistant response becomes its run output. Absent defaults to
   * `"none"`, so a node withholds its output unless opted in.
   */
  /** Optional structured parsing performed in addition to the stable raw output. */
  outputContract?: WorkflowAgentOutputContract;
}

/** Serializable workflow node data shared by memory and future Rust adapters. */
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
  /** Legacy executable cases retained so existing saved graphs remain readable. */
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
  /** Fixture-only timing hint; the editor does not expose it as configuration. */
  mockStepMs?: number;
}

export interface WorkflowPosition {
  x: number;
  y: number;
}

export interface WorkflowViewport extends WorkflowPosition {
  zoom: number;
}

/** Serializable node snapshot; React Flow runtime internals are intentionally excluded. */
export interface WorkflowDefinitionNode {
  id: string;
  type: "workflow";
  position: WorkflowPosition;
  data: WorkflowNodeData;
  deletable?: boolean;
  initialWidth?: number;
  initialHeight?: number;
}

/** Serializable execution edge with display text kept as plain data. */
export interface WorkflowDefinitionEdge {
  id: string;
  source: string;
  target: string;
  type?: "workflow";
  label?: string;
  data?: Record<string, unknown>;
  /** Source port of a Condition node selecting its branch: a case id or `else`. */
  sourceHandle?: string;
  targetHandle?: string;
}

/** Frozen workflow definition shared across memory and future generated contracts. */
export interface WorkflowDefinition {
  id: string;
  name: string;
  description: string;
  updatedAt: string;
  viewport: WorkflowViewport;
  globalVariables?: WorkflowGlobalVariable[];
  nodes: WorkflowDefinitionNode[];
  edges: WorkflowDefinitionEdge[];
}

/** Run-level lifecycle for a project-attached graph workflow execution. */
export type GraphWorkflowRunStatus =
  | "pending"
  | "running"
  | "awaiting_input"
  | "succeeded"
  | "failed"
  | "cancelled";

/** Per-node execution status overlaid on a frozen definition snapshot. */
export type GraphWorkflowNodeStatus =
  | "idle"
  | "inactive"
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "awaiting_input";

/** HITL timeout policy; MVP mock always waits (`wait`) until submit. */
export type HitlTimeoutPolicy = "fail" | "skip" | "wait";

/** Glanceable runtime I/O for Theater inspector (not raw wire frames). */
export interface GraphWorkflowNodeIo {
  /** One-line summary shown by default. */
  summary: string;
  /** Optional longer body (prompt, tool args, model reply, HITL answer…). */
  detail?: string;
}

/** One file this node incrementally changed, recorded from the worktree git diff. */
export interface WorkflowNodeFileChange {
  /** Worktree-relative file path. */
  path: string;
  additions: number;
  deletions: number;
}

export interface GraphWorkflowNodeState {
  status: GraphWorkflowNodeStatus;
  /** Session bound to this node execution; opaque to the workflow UI. */
  sessionId?: string;
  startedAt?: string;
  finishedAt?: string;
  errorMessage?: string;
  /** ACP stop reason recorded in `payload.stop_reason` when the node succeeded. */
  stopReason?: string;
  /** What this step received when it started (kickoff, upstream, schema…). */
  input?: GraphWorkflowNodeIo;
  /** What this step produced when it finished (or HITL answer summary). */
  output?: GraphWorkflowNodeIo;
  /** The node's own conversation, projected from its run output by the real adapter. */
  conversation?: WorkflowNodeConversationItem[];
  /** Incremental worktree changes recorded in `payload.file_changes`. */
  fileChanges?: WorkflowNodeFileChange[];
}

/** Lifecycle state for one projected session item. */
export type WorkflowNodeConversationItemStatus = "streaming" | "complete";

/** Roles used by visible text messages in a node-bound session. */
export type WorkflowNodeConversationMessageRole = "user" | "assistant";

/** Kinds of secondary session activity that can be disclosed on demand. */
export type WorkflowNodeConversationActivityKind = "thought" | "tool";

interface WorkflowNodeConversationItemBase {
  id: string;
  runId: string;
  nodeId: string;
  sessionId: string;
  createdAt: string;
  updatedAt: string;
}

/** A user or Agent message approved for the compact node conversation. */
export interface WorkflowNodeConversationMessage extends WorkflowNodeConversationItemBase {
  kind: "message";
  role: WorkflowNodeConversationMessageRole;
  markdown: string;
  status: WorkflowNodeConversationItemStatus;
}

/** A raw-session item retained for an explicitly expanded activity disclosure. */
export interface WorkflowNodeConversationActivity extends WorkflowNodeConversationItemBase {
  kind: "activity";
  activityKind: WorkflowNodeConversationActivityKind;
  summary: string;
  detail?: string;
  status: WorkflowNodeConversationItemStatus;
}

/**
 * Filtered session projection used by workflow cards.
 *
 * Adapters may retain thoughts and tool calls as activity items, but the card
 * renders them behind one collapsed disclosure while showing text messages in
 * the same layout as the full chat surface.
 */
export type WorkflowNodeConversationItem =
  WorkflowNodeConversationMessage | WorkflowNodeConversationActivity;

/**
 * A project-scoped execution of a mounted workflow definition.
 */
export interface GraphWorkflowRun {
  id: string;
  projectId: string;
  definitionId: string;
  definitionSnapshot: WorkflowDefinition;
  name: string;
  status: GraphWorkflowRunStatus;
  kickoffInput?: string;
  nodeStates: Record<string, GraphWorkflowNodeState>;
  /** Open HITL gates (parallel prompts may all wait at once). Cleared on resolve / cancel. */
  openHitls: HitlRequest[];
  createdAt: string;
  updatedAt: string;
  finishedAt?: string;
}

/**
 * Pending-only overrides on a run's frozen node copy.
 * Never written back to the mounted library definition.
 */
export interface GraphWorkflowSnapshotNodePatch {
  input?: string;
  instruction?: string;
  description?: string;
}

/** Reference mount: many projects may point at the same definition id. */
export interface ProjectWorkflowMount {
  projectId: string;
  definitionId: string;
  definitionName: string;
  mountedAt: string;
}

export type WorkflowRunEvent =
  | { type: "run_started"; runId: string }
  | { type: "node_started"; runId: string; nodeId: string }
  | {
      type: "node_finished";
      runId: string;
      nodeId: string;
      status: GraphWorkflowNodeStatus;
    }
  | {
      type: "artifact_added";
      runId: string;
      artifact: WorkflowArtifact;
    }
  | {
      type: "node_conversation_item_upserted";
      runId: string;
      item: WorkflowNodeConversationItem;
    }
  | { type: "hitl_required"; runId: string; request: HitlRequest }
  | {
      type: "hitl_resolved";
      runId: string;
      requestId: string;
      nodeId: string;
      /** Submitted field values (keys = field.name). */
      payload: Record<string, unknown>;
    }
  | { type: "run_finished"; runId: string; status: GraphWorkflowRunStatus };

/** Opaque resume marker returned to a future Desktop channel adapter unchanged. */
export type WorkflowEventCursor = string;

/** Durable event metadata used for ordering, deduplication, and reconnect. */
export type WorkflowRunEventEnvelope = WorkflowRunEvent & {
  cursor: WorkflowEventCursor;
  sequence: number;
  occurredAt: string;
};

/** Consistent initial state paired with the cursor from which streaming resumes. */
export interface WorkflowRunLiveSnapshot {
  run: GraphWorkflowRun;
  artifacts: WorkflowArtifact[];
  /** Filtered node session projection; activity stays available for disclosure. */
  conversation: WorkflowNodeConversationItem[];
  cursor: WorkflowEventCursor | null;
}

export type WorkflowArtifactKind = "text" | "markdown" | "file" | "diff";

export interface WorkflowArtifact {
  id: string;
  runId: string;
  nodeId: string;
  kind: WorkflowArtifactKind;
  title: string;
  body: string;
  createdAt: string;
}

/** Field types shared by HITL (and future Kickoff schema) forms. */
export type HitlFieldType = "text" | "textarea" | "select";

/**
 * Why the engine paused for a human.
 * - `approval` — permission / scope choice
 * - `feedback` — free-form notes to continue
 * - `clarify` — model/engine question the user must answer
 */
export type HitlGateKind = "approval" | "feedback" | "clarify";

export interface HitlFieldOption {
  value: string;
  label: string;
}

export interface HitlField {
  name: string;
  type: HitlFieldType;
  label: string;
  required?: boolean;
  placeholder?: string;
  options?: HitlFieldOption[];
}

export interface HitlSchema {
  kind: HitlGateKind;
  title?: string;
  /** Model/engine question or instruction body (plain text). */
  prompt?: string;
  fields: HitlField[];
}

export interface HitlRequest {
  id: string;
  runId: string;
  nodeId: string;
  schema: HitlSchema;
  /**
   * When false, UI should not treat the run as modal-locked (browse OK).
   * Mock engine still pauses scheduling for MVP when any blocking gate is open.
   */
  blocking: boolean;
  timeoutAt?: string;
  policy: HitlTimeoutPolicy;
  status: "open" | "resolved" | "timed_out";
  /** Local-time ISO when the gate opened (reconnect / ordering). */
  createdAt: string;
}

/** Open gates still waiting for the user. */
export function listOpenHitls(run: GraphWorkflowRun): HitlRequest[] {
  return run.openHitls.filter((request) => request.status === "open");
}

/** Finds an open gate by id, or undefined when missing / already resolved. */
export function findOpenHitl(
  run: GraphWorkflowRun,
  requestId: string,
): HitlRequest | undefined {
  return listOpenHitls(run).find((request) => request.id === requestId);
}

/** Finds an open gate for a node, if any. */
export function findOpenHitlForNode(
  run: GraphWorkflowRun,
  nodeId: string,
): HitlRequest | undefined {
  return listOpenHitls(run).find((request) => request.nodeId === nodeId);
}

export type Unsubscribe = () => void;
