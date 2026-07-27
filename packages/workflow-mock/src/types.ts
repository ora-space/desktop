export type WorkflowNodeKind =
  | "start"
  | "prompt"
  | "agent"
  | "condition"
  | "tool"
  | "output";

export type WorkflowLocale = "zh-CN" | "en-US";

export interface WorkflowPosition {
  x: number;
  y: number;
}

export interface WorkflowNodeConfig {
  instruction: string;
  model?: string;
  tool?: string;
  condition?: string;
}

export interface WorkflowNode {
  id: string;
  kind: WorkflowNodeKind;
  title: string;
  description: string;
  position: WorkflowPosition;
  config: WorkflowNodeConfig;
}

export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
  label?: string;
}

export interface WorkflowDefinition {
  id: string;
  name: string;
  description: string;
  updatedAt: string;
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
}

export type WorkflowRunStatus = "success" | "failed";

export interface WorkflowRunStep {
  nodeId: string;
  durationMs: number;
  summary: string;
}

export interface WorkflowRunResult {
  status: WorkflowRunStatus;
  durationMs: number;
  output: string;
  steps: WorkflowRunStep[];
}

/** Defines the async boundary the real workflow backend can implement later. */
export interface WorkflowRepository {
  list(): Promise<WorkflowDefinition[]>;
  get(id: string): Promise<WorkflowDefinition>;
  save(workflow: WorkflowDefinition): Promise<WorkflowDefinition>;
  run(id: string, input: string): Promise<WorkflowRunResult>;
}
