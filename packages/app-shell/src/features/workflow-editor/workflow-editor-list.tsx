import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkflowLibrary } from "./workflow-definitions";
import { useWorkflowEditorStore } from "./workflow-editor-store";
import { WorkflowManager } from "./workflow-manager";

/**
 * Sidebar body for editor mode: the persisted library, wired through the
 * editor's registered flush-and-switch actions so unsaved drafts are not dropped.
 */
export function WorkflowEditorList() {
  const { t } = useTranslation();
  const library = useWorkflowLibrary();
  const selectedWorkflowId = useWorkflowEditorStore(
    (state) => state.selectedWorkflowId,
  );
  const managerError = useWorkflowEditorStore((state) => state.managerError);
  const actions = useWorkflowEditorStore((state) => state.actions);
  const sidebarCollapsed = useUiStore((state) => state.sidebarCollapsed);
  const libraryWorkflows = useMemo(
    () =>
      (library.data ?? []).map((summary) => ({
        id: summary.id,
        name: summary.name,
      })),
    [library.data],
  );
  const libraryError =
    library.error !== null ? localizeContractError(library.error, t) : null;
  // When the rail is collapsed these messages move to the editor chrome so they
  // stay visible; showing both would duplicate the same alert.
  const error = sidebarCollapsed ? null : (managerError ?? libraryError);

  return (
    <WorkflowManager
      workflows={libraryWorkflows}
      selectedWorkflowId={selectedWorkflowId}
      error={error}
      disabled={actions === null}
      onSelect={(workflowId) => {
        // Never switch identity without the editor's flush path — a store-only
        // write would drop unsaved canvas edits if the editor is still mounted.
        if (actions !== null) void actions.select(workflowId);
      }}
      onCreate={(name) =>
        actions === null ? Promise.resolve(false) : actions.create(name)
      }
      onCopy={(workflowId) =>
        actions === null ? Promise.resolve(false) : actions.copy(workflowId)
      }
      onRename={(workflowId, name) =>
        actions === null
          ? Promise.resolve(false)
          : actions.rename(workflowId, name)
      }
      onDelete={(workflowId) => {
        if (actions !== null) void actions.delete(workflowId);
      }}
      onImport={(file) =>
        actions === null ? Promise.resolve(false) : actions.importFile(file)
      }
    />
  );
}
