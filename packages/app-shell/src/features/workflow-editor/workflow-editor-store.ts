import { create } from "zustand";

/**
 * Library mutations that must run through the open editor so a draft flush
 * happens before the selected workflow identity changes.
 */
export interface WorkflowEditorLibraryActions {
  select: (workflowId: string) => Promise<void>;
  create: (name: string) => Promise<boolean>;
  copy: (workflowId: string) => Promise<boolean>;
  rename: (workflowId: string, name: string) => Promise<boolean>;
  delete: (workflowId: string) => Promise<void>;
  /** Opens the import dialog; file selection and preview live in the editor. */
  openImport: () => void;
  /** Selects the workflow (flushing the open draft) and opens its export dialog. */
  exportFile: (workflowId: string) => Promise<void>;
  leave: () => Promise<void>;
}

interface WorkflowEditorState {
  selectedWorkflowId: string | null;
  managerError: string | null;
  actions: WorkflowEditorLibraryActions | null;
  /** Only a failed, unmounted editor can switch without flushing its draft. */
  renderRecovery: {
    status: "ready" | "failed" | "recovering";
    revision: number;
  };
  /** Workflows imported in this session, marked in the library until reload. */
  importedWorkflowIds: readonly string[];
  markImported: (workflowId: string) => void;
  setSelectedWorkflowId: (selectedWorkflowId: string | null) => void;
  setManagerError: (managerError: string | null) => void;
  registerActions: (actions: WorkflowEditorLibraryActions | null) => void;
  reportRenderFailure: () => void;
  selectAfterRenderFailure: (workflowId: string) => void;
}

/**
 * Session-only editor selection shared by the sidebar list and the canvas.
 * Not persisted: leaving the surface or reloading returns to the parked chat.
 */
export const useWorkflowEditorStore = create<WorkflowEditorState>((set) => ({
  selectedWorkflowId: null,
  managerError: null,
  actions: null,
  renderRecovery: { status: "ready", revision: 0 },
  importedWorkflowIds: [],
  markImported: (workflowId) =>
    set((state) => ({
      importedWorkflowIds: state.importedWorkflowIds.includes(workflowId)
        ? state.importedWorkflowIds
        : [...state.importedWorkflowIds, workflowId],
    })),
  setSelectedWorkflowId: (selectedWorkflowId) => set({ selectedWorkflowId }),
  setManagerError: (managerError) => set({ managerError }),
  registerActions: (actions) =>
    set((state) => ({
      actions,
      renderRecovery:
        actions === null
          ? state.renderRecovery
          : { ...state.renderRecovery, status: "ready" },
    })),
  reportRenderFailure: () =>
    set((state) => ({
      renderRecovery: { ...state.renderRecovery, status: "failed" },
    })),
  selectAfterRenderFailure: (workflowId) =>
    set((state) => {
      if (state.actions !== null || state.renderRecovery.status !== "failed") {
        return state;
      }
      // The boundary has already unmounted the failed editor and disposed its
      // autosave lifecycle. A new selection now starts a fresh draft session.
      return {
        selectedWorkflowId: workflowId,
        managerError: null,
        renderRecovery: {
          status: "recovering",
          revision: state.renderRecovery.revision + 1,
        },
      };
    }),
}));

/** Connects the workspace boundary to editor-owned recovery without changing healthy selections. */
export function useWorkflowEditorRenderRecovery() {
  const revision = useWorkflowEditorStore(
    (state) => state.renderRecovery.revision,
  );
  const onRenderFailure = useWorkflowEditorStore(
    (state) => state.reportRenderFailure,
  );
  return { revision, onRenderFailure };
}
