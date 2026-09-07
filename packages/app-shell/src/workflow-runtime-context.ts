import { createContext, useContext } from "react";
import type { WorkflowRuntime } from "@ora/workflow-runtime";

export const WorkflowRuntimeContext = createContext<WorkflowRuntime | null>(
  null,
);

/** Active workflow runtime (host mounts + graph runs). */
export function useWorkflowRuntime(): WorkflowRuntime {
  const runtime = useContext(WorkflowRuntimeContext);
  if (runtime === null) {
    throw new Error("useWorkflowRuntime requires WorkflowRuntimeProvider");
  }
  return runtime;
}
