import { createContext, useContext, type ReactNode } from "react";

export interface WorkflowFlowCallbacks {
  connectionCandidateEndpoint?: "source" | "target" | null;
  connectionCandidateNodeId?: string | null;
  onDeleteNode: (nodeId: string) => void;
  onDeleteEdge: (edgeId: string) => void;
  onSelectEdge: (edgeId: string) => void;
}

const WorkflowFlowCallbacksContext = createContext<WorkflowFlowCallbacks | null>(null);

/** Provides canvas mutation callbacks to custom React Flow node/edge renderers. */
export function WorkflowFlowCallbacksProvider({
  value,
  children,
}: {
  value: WorkflowFlowCallbacks;
  children: ReactNode;
}) {
  return (
    <WorkflowFlowCallbacksContext.Provider value={value}>
      {children}
    </WorkflowFlowCallbacksContext.Provider>
  );
}

/** Reads canvas callbacks; custom nodes must render inside the provider. */
export function useWorkflowFlowCallbacks(): WorkflowFlowCallbacks {
  const value = useContext(WorkflowFlowCallbacksContext);
  if (value === null) {
    throw new Error("useWorkflowFlowCallbacks requires WorkflowFlowCallbacksProvider");
  }
  return value;
}
