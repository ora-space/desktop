import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkflowLibrary } from "../../state/data/workflows";
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
  const renderFailed = useWorkflowEditorStore(
    (state) => state.renderRecovery.status === "failed",
  );
  const selectAfterRenderFailure = useWorkflowEditorStore(
    (state) => state.selectAfterRenderFailure,
  );
  const importedWorkflowIds = useWorkflowEditorStore(
    (state) => state.importedWorkflowIds,
  );
  const sidebarCollapsed = useUiStore((state) => state.sidebarCollapsed);
  const libraryWorkflows = useMemo(
    () =>
      (library.data ?? []).map((summary) => ({
        id: summary.id,
        name: summary.name,
        imported: importedWorkflowIds.includes(summary.id),
      })),
    [importedWorkflowIds, library.data],
  );
  const libraryError =
    library.error !== null ? localizeContractError(library.error, t) : null;
  // When the rail is collapsed these messages move to the editor chrome so they
  // stay visible; showing both would duplicate the same alert.
  const error = sidebarCollapsed ? null : (managerError ?? libraryError);

  return (
    <WorkflowManager
      workflows={libraryWorkflows}
      libraryLoaded={library.data !== undefined}
      selectedWorkflowId={selectedWorkflowId}
      error={error}
      disabled={actions === null}
      selectionDisabled={actions === null && !renderFailed}
      onSelect={(workflowId) => {
        // Mounted editors always flush before switching. Only a render failure
        // that has unmounted the editor permits the recovery selection path.
        if (actions !== null) {
          void actions.select(workflowId);
        } else {
          selectAfterRenderFailure(workflowId);
        }
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
      onImport={() => actions?.openImport()}
      onExport={(workflowId) => {
        if (actions !== null) void actions.exportFile(workflowId);
      }}
    />
  );
}
