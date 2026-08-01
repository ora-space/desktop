import { createContext, useContext, type ReactNode } from "react";

interface WorkflowConnectionState {
  connectionCandidateEndpoint?: "source" | "target" | null;
  connectionCandidateNodeId?: string | null;
}

const WorkflowConnectionStateContext =
  createContext<WorkflowConnectionState | null>(null);

/** Provides candidate feedback for the editor's whole-card connection target. */
export function WorkflowConnectionStateProvider({
  value,
  children,
}: {
  value: WorkflowConnectionState;
  children: ReactNode;
}) {
  return (
    <WorkflowConnectionStateContext.Provider value={value}>
      {children}
    </WorkflowConnectionStateContext.Provider>
  );
}

/** Reads transient state used only by the custom whole-card connection behavior. */
export function useWorkflowConnectionState(): WorkflowConnectionState {
  const value = useContext(WorkflowConnectionStateContext);
  if (value === null) {
    throw new Error("useWorkflowConnectionState requires WorkflowConnectionStateProvider");
  }
  return value;
}
