import type { DemoWorkflow } from "@ora/workflow-mock";

/** Run-level lifecycle for a project-attached graph workflow execution. */
export type GraphWorkflowRunStatus =
  | "pending"
  | "running"
  | "awaiting_input"
  | "succeeded"
  | "partial_failed"
  | "failed"
  | "cancelled";

/** Per-node execution status overlaid on a frozen definition snapshot. */
export type GraphWorkflowNodeStatus =
  | "idle"
  | "running"
  | "succeeded"
  | "failed"
  | "skipped"
  | "cancelled"
  | "awaiting_input";

/** HITL timeout policy; MVP mock implements `fail` only. */
export type HitlTimeoutPolicy = "fail" | "skip" | "wait";

export interface GraphWorkflowTokenUsage {
  inputTokens?: number;
  outputTokens?: number;
  totalTokens?: number;
}

export interface GraphWorkflowNodeState {
  status: GraphWorkflowNodeStatus;
  startedAt?: string;
  finishedAt?: string;
  durationMs?: number;
  tokenUsage?: GraphWorkflowTokenUsage;
  errorMessage?: string;
}

/**
 * A project-scoped execution of a mounted workflow definition.
 * Named GraphWorkflowRun so it never collides with OpenSpec WorkflowRun.
 */
export interface GraphWorkflowRun {
  id: string;
  projectId: string;
  definitionId: string;
  definitionSnapshot: DemoWorkflow;
  name: string;
  status: GraphWorkflowRunStatus;
  kickoffInput?: string;
  nodeStates: Record<string, GraphWorkflowNodeState>;
  totals: {
    durationMs?: number;
    tokenUsage?: GraphWorkflowTokenUsage;
  };
  createdAt: string;
  updatedAt: string;
  finishedAt?: string;
}

/**
 * Pending-only overrides on a run's frozen node copy.
 * Never written back to the mounted library definition.
 */
export interface GraphWorkflowSnapshotNodePatch {
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
      durationMs?: number;
      tokenUsage?: GraphWorkflowTokenUsage;
    }
  | {
      type: "artifact_added";
      runId: string;
      artifact: WorkflowArtifact;
    }
  | { type: "hitl_required"; runId: string; request: HitlRequest }
  | { type: "hitl_resolved"; runId: string; requestId: string }
  | {
      type: "run_finished";
      runId: string;
      status: GraphWorkflowRunStatus;
      totals: GraphWorkflowRun["totals"];
    };

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

export interface HitlRequest {
  id: string;
  runId: string;
  nodeId: string;
  /** JSON-schema-like field list; renderer arrives in Step 5. */
  schema: Record<string, unknown>;
  timeoutAt?: string;
  policy: HitlTimeoutPolicy;
  status: "open" | "resolved" | "timed_out";
}

export type Unsubscribe = () => void;
