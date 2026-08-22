import { toast } from "@ora/ui";
import {
  useComposerFileContextStore,
  type ComposerFileSelection,
} from "../../state/stores/composer-file-context-store";
import { conversationKeyFor } from "../../state/stores/conversation-key";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { appI18n } from "../../i18n/i18n-instance";

/** Queues one file line quote for the active conversation composer. */
export function addComposerFileSelection(
  selection: ComposerFileSelection,
): boolean {
  return addComposerFileSelections([selection]);
}

/**
 * Queues several quotes as one batch (e.g. file-preview gaps). Returns true
 * when at least one selection was newly accepted.
 */
export function addComposerFileSelections(
  selections: ComposerFileSelection[],
): boolean {
  if (selections.length === 0) return false;
  const conversationKey = conversationKeyFor(
    useWorkspaceSelectionStore.getState().selection,
  );
  if (conversationKey === "__none__") {
    toast.warning(appI18n.t("files.lineSelectionNeedsChat"));
    return false;
  }
  return useComposerFileContextStore
    .getState()
    .addSelections(conversationKey, selections);
}
