import type { Edge, Node } from "@xyflow/react";
import type {
  DemoWorkflow,
  WorkflowAnnotationNode,
  WorkflowGlobalVariable,
  WorkflowNodeData,
} from "@ora/workflow-mock";

/** Identifies the user-visible operation that produced a history step. */
export type WorkflowHistoryEvent =
  | "node.add"
  | "annotation.add"
  | "node.delete"
  | "edge.delete"
  | "edge.connect"
  | "edge.reconnect"
  | "node.move"
  | "layout.organize"
  | "node.edit"
  | "annotation.edit"
  | "workflow.variables"
  | "workflow.rename";

/** Stores the workflow fields that represent authored content, excluding UI state. */
export interface WorkflowHistorySnapshot {
  name: string;
  description: string;
  nodes: Node<WorkflowNodeData, "workflow">[];
  edges: Edge[];
  annotations: WorkflowAnnotationNode[];
  globalVariables?: WorkflowGlobalVariable[];
}

/** Adds stable context to a history step without coupling the history engine to UI text. */
export interface WorkflowHistoryMeta {
  nodeIds?: string[];
  edgeIds?: string[];
  /** Human-readable affected workflow elements, captured at the edit boundary. */
  subject?: string;
  nodeTitle?: string;
  nodeKind?: string;
}

/** Represents one completed authored edit and the state immediately before it. */
export interface WorkflowHistoryStep {
  id: string;
  event: WorkflowHistoryEvent;
  meta?: WorkflowHistoryMeta;
  snapshot: WorkflowHistorySnapshot;
  fingerprint: string;
}

/** Holds the linear undo and redo stacks for one editor session. */
export interface WorkflowHistoryState {
  past: WorkflowHistoryStep[];
  future: WorkflowHistoryStep[];
}

/** Creates an empty history for a newly mounted workflow editor. */
export function createWorkflowHistoryState(): WorkflowHistoryState {
  return { past: [], future: [] };
}

/** Removes React Flow selection and measurement state before a snapshot is retained. */
export function captureWorkflowHistorySnapshot(
  workflow: DemoWorkflow,
): WorkflowHistorySnapshot {
  const snapshot: WorkflowHistorySnapshot = {
    name: workflow.name,
    description: workflow.description,
    nodes: workflow.nodes.map((node) => ({
      ...node,
      selected: false,
      data: { ...node.data },
    })),
    edges: workflow.edges.map((edge) => ({
      ...edge,
      selected: false,
    })),
    annotations: (workflow.annotations ?? []).map((annotation) => ({
      ...annotation,
      selected: false,
      data: { ...annotation.data },
    })),
    globalVariables: structuredClone(workflow.globalVariables ?? []),
  };
  return structuredClone(snapshot);
}

/** Produces a deterministic comparison key for semantic workflow content. */
export function workflowHistoryFingerprint(
  snapshot: WorkflowHistorySnapshot,
): string {
  return JSON.stringify(snapshot);
}

/** Restores authored fields while preserving the active workflow identity and viewport. */
export function restoreWorkflowHistorySnapshot(
  workflow: DemoWorkflow,
  snapshot: WorkflowHistorySnapshot,
): DemoWorkflow {
  return {
    ...workflow,
    name: snapshot.name,
    description: snapshot.description,
    nodes: snapshot.nodes.map((node) => ({ ...node, selected: false })),
    edges: snapshot.edges.map((edge) => ({ ...edge, selected: false })),
    annotations: snapshot.annotations.map((annotation) => ({
      ...annotation,
      selected: false,
    })),
    globalVariables: structuredClone(snapshot.globalVariables ?? []),
  };
}

/** Records a completed edit, ignoring semantic no-ops and invalidating redo history. */
export function commitWorkflowHistory(
  state: WorkflowHistoryState,
  before: WorkflowHistorySnapshot,
  after: WorkflowHistorySnapshot,
  event: WorkflowHistoryEvent,
  meta?: WorkflowHistoryMeta,
  id = `${Date.now()}-${Math.random().toString(36).slice(2)}`,
): WorkflowHistoryState {
  const beforeFingerprint = workflowHistoryFingerprint(before);
  if (beforeFingerprint === workflowHistoryFingerprint(after)) {
    return state;
  }
  const past = [
    ...state.past,
    { id, event, meta, snapshot: before, fingerprint: beforeFingerprint },
  ];
  return {
    past: past.slice(-50),
    future: [],
  };
}

/** Moves one step backward and returns the snapshot that should become current. */
export function undoWorkflowHistory(
  state: WorkflowHistoryState,
  current: WorkflowHistorySnapshot,
): { state: WorkflowHistoryState; snapshot: WorkflowHistorySnapshot | null } {
  const step = state.past.at(-1);
  if (step === undefined) {
    return { state, snapshot: null };
  }
  return {
    state: {
      past: state.past.slice(0, -1),
      future: [
        ...state.future,
        {
          ...step,
          snapshot: current,
          fingerprint: workflowHistoryFingerprint(current),
        },
      ],
    },
    snapshot: step.snapshot,
  };
}

/** Moves one step forward and returns the snapshot that should become current. */
export function redoWorkflowHistory(
  state: WorkflowHistoryState,
  current: WorkflowHistorySnapshot,
): { state: WorkflowHistoryState; snapshot: WorkflowHistorySnapshot | null } {
  const step = state.future.at(-1);
  if (step === undefined) {
    return { state, snapshot: null };
  }
  return {
    state: {
      past: [
        ...state.past,
        {
          ...step,
          snapshot: current,
          fingerprint: workflowHistoryFingerprint(current),
        },
      ],
      future: state.future.slice(0, -1),
    },
    snapshot: step.snapshot,
  };
}
