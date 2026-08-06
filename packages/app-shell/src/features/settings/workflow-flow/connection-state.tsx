import type { ReactNode } from "react";
import {
  WorkflowConnectionStateContext,
  type WorkflowConnectionState,
} from "./use-connection-state";

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
