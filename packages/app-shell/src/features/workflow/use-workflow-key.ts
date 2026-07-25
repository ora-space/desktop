import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { workflowKeyFor } from "../../state/stores/workflow-store";

/** The workflow store key for the current selection (session id, or a task key). */
export function useWorkflowKey(): string {
  return useWorkspaceSelectionStore((state) => workflowKeyFor(state.selection));
}
