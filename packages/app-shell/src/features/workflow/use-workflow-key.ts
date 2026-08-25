import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { conversationKeyFor } from "../../state/stores/conversation-key";

/** The workflow store key for the current selection's session or Workspace surface. */
export function useWorkflowKey(): string {
  return useWorkspaceSelectionStore((state) =>
    conversationKeyFor(state.selection),
  );
}
