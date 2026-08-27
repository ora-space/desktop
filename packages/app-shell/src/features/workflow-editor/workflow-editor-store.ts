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
  importFile: (file: File) => Promise<boolean>;
  leave: () => Promise<void>;
}

interface WorkflowEditorState {
  selectedWorkflowId: string | null;
  managerError: string | null;
  actions: WorkflowEditorLibraryActions | null;
  setSelectedWorkflowId: (selectedWorkflowId: string | null) => void;
  setManagerError: (managerError: string | null) => void;
  registerActions: (actions: WorkflowEditorLibraryActions | null) => void;
}

/**
 * Session-only editor selection shared by the sidebar list and the canvas.
 * Not persisted: leaving the surface or reloading returns to the parked chat.
 */
export const useWorkflowEditorStore = create<WorkflowEditorState>((set) => ({
  selectedWorkflowId: null,
  managerError: null,
  actions: null,
  setSelectedWorkflowId: (selectedWorkflowId) => set({ selectedWorkflowId }),
  setManagerError: (managerError) => set({ managerError }),
  registerActions: (actions) => set({ actions }),
}));
