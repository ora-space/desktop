import { create } from "zustand";

export interface ComposerFileSelection {
  path: string;
  startLine: number;
  endLine: number;
}

interface PendingFileContext {
  id: number;
  selections: ComposerFileSelection[];
}

interface ComposerFileContextState {
  pendingByTask: Record<string, PendingFileContext | undefined>;
  /** Queues a workspace-relative line range for the composer belonging to one task. */
  addSelection: (taskId: string, selection: ComposerFileSelection) => void;
  /** Removes a request only when it is the request the composer consumed. */
  consumeSelections: (taskId: string, requestId: number) => void;
}

let nextRequestId = 0;

/** Bridges file-explorer actions to the task composer without coupling the two views. */
export const useComposerFileContextStore = create<ComposerFileContextState>((set) => ({
  pendingByTask: {},
  addSelection: (taskId, selection) => {
    set((state) => {
      const pending = state.pendingByTask[taskId];
      const existingSelections = pending?.selections ?? [];
      const alreadyQueued = existingSelections.some((candidate) =>
        candidate.path === selection.path
        && candidate.startLine === selection.startLine
        && candidate.endLine === selection.endLine,
      );
      if (alreadyQueued) return state;
      const selections = [...existingSelections, selection];
      return {
        pendingByTask: {
          ...state.pendingByTask,
          [taskId]: { id: ++nextRequestId, selections },
        },
      };
    });
  },
  consumeSelections: (taskId, requestId) => {
    set((state) => {
      const pending = state.pendingByTask[taskId];
      if (pending?.id !== requestId) return state;
      const pendingByTask = { ...state.pendingByTask };
      delete pendingByTask[taskId];
      return { pendingByTask };
    });
  },
}));
