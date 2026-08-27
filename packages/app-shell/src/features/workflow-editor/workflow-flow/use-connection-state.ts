import { createContext, useContext } from "react";

export interface WorkflowConnectionState {
  connectionCandidateEndpoint?: "source" | "target" | null;
  connectionCandidateNodeId?: string | null;
}

export const WorkflowConnectionStateContext =
  createContext<WorkflowConnectionState | null>(null);

/** Reads transient state used only by the custom whole-card connection behavior. */
export function useWorkflowConnectionState(): WorkflowConnectionState {
  const value = useContext(WorkflowConnectionStateContext);
  if (value === null) {
    throw new Error(
      "useWorkflowConnectionState requires WorkflowConnectionStateProvider",
    );
  }
  return value;
}
