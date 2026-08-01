import { createContext, useContext, useMemo, type ReactNode } from "react";

export interface WorkflowFlowActions {
  onDeleteNode: (nodeId: string) => void;
  onDeleteEdge: (edgeId: string) => void;
  onSelectEdge: (edgeId: string) => void;
}

export interface WorkflowFlowConnectionState {
  connectionCandidateEndpoint?: "source" | "target" | null;
  connectionCandidateNodeId?: string | null;
}

export type WorkflowFlowCallbacks = WorkflowFlowActions & WorkflowFlowConnectionState;

const WorkflowFlowActionsContext = createContext<WorkflowFlowActions | null>(null);
const WorkflowFlowConnectionStateContext =
  createContext<WorkflowFlowConnectionState | null>(null);

/** Separates stable graph actions from pointer-frequency connection state. */
export function WorkflowFlowCallbacksProvider({
  value,
  children,
}: {
  value: WorkflowFlowCallbacks;
  children: ReactNode;
}) {
  const actions = useMemo<WorkflowFlowActions>(() => ({
    onDeleteNode: value.onDeleteNode,
    onDeleteEdge: value.onDeleteEdge,
    onSelectEdge: value.onSelectEdge,
  }), [value.onDeleteEdge, value.onDeleteNode, value.onSelectEdge]);
  const connectionState = useMemo<WorkflowFlowConnectionState>(() => ({
    connectionCandidateEndpoint: value.connectionCandidateEndpoint,
    connectionCandidateNodeId: value.connectionCandidateNodeId,
  }), [value.connectionCandidateEndpoint, value.connectionCandidateNodeId]);

  return (
    <WorkflowFlowActionsContext.Provider value={actions}>
      <WorkflowFlowConnectionStateContext.Provider value={connectionState}>
        {children}
      </WorkflowFlowConnectionStateContext.Provider>
    </WorkflowFlowActionsContext.Provider>
  );
}

/** Reads stable graph actions without subscribing an edge to drag-preview state. */
export function useWorkflowFlowActions(): WorkflowFlowActions {
  const value = useContext(WorkflowFlowActionsContext);
  if (value === null) {
    throw new Error("useWorkflowFlowActions requires WorkflowFlowCallbacksProvider");
  }
  return value;
}

/** Reads transient connection state only where candidate feedback is rendered. */
export function useWorkflowFlowConnectionState(): WorkflowFlowConnectionState {
  const value = useContext(WorkflowFlowConnectionStateContext);
  if (value === null) {
    throw new Error("useWorkflowFlowConnectionState requires WorkflowFlowCallbacksProvider");
  }
  return value;
}
