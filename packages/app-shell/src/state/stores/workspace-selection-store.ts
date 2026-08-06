import { create } from "zustand";

export interface WorkspaceSelection {
  projectId: string | null;
  taskId: string | null;
  sessionId: string | null;
  /** Graph workflow run; mutually exclusive with task/session legs. */
  workflowRunId: string | null;
}

interface WorkspaceSelectionState {
  selection: WorkspaceSelection;
  /** Selects a project and clears any task/session/run underneath. */
  selectProject: (projectId: string) => void;
  /** Selects a task under a project and clears any session/run underneath. */
  selectTask: (taskId: string, projectId: string) => void;
  /**
   * Selects a session whose owning task is still being created.
   *
   * A direct chat gets its session before its task, because the task's title
   * comes from the first message. The session id is already final, so this only
   * leaves the task leg empty until the task exists.
   */
  selectSessionBeforeTask: (sessionId: string, projectId: string) => void;
  /** Selects a specific session, recording its owning task and project. */
  selectSession: (sessionId: string, taskId: string, projectId: string) => void;
  /** Selects a graph workflow run under a project (clears task/session). */
  selectWorkflowRun: (workflowRunId: string, projectId: string) => void;
  /** Clears the entire selection. */
  clearSelection: () => void;
  /** Clears only the session leg (used after the selected session is deleted). */
  clearSessionSelection: () => void;
  /** Clears the task and session legs (used after the selected task is deleted). */
  clearTaskSelection: (projectId: string) => void;
  /** Clears the workflow-run leg while keeping the project. */
  clearWorkflowRunSelection: (projectId: string) => void;
  /** Replaces the project leg, clearing task/session/run (used after the selected project is deleted). */
  setProject: (projectId: string | null) => void;
}

const EMPTY_SELECTION: WorkspaceSelection = {
  projectId: null,
  taskId: null,
  sessionId: null,
  workflowRunId: null,
};

/**
 * Owns the workspace tree selection without coupling to query data. Callers pass
 * the owning project/task ids they already have from react-query results, which
 * keeps this store a pure state machine that is trivial to unit-test.
 */
export const useWorkspaceSelectionStore = create<WorkspaceSelectionState>((set) => ({
  selection: EMPTY_SELECTION,
  selectProject: (projectId) =>
    set({
      selection: {
        projectId,
        taskId: null,
        sessionId: null,
        workflowRunId: null,
      },
    }),
  selectTask: (taskId, projectId) =>
    set({
      selection: {
        projectId,
        taskId,
        sessionId: null,
        workflowRunId: null,
      },
    }),
  selectSessionBeforeTask: (sessionId, projectId) =>
    set({
      selection: {
        projectId,
        taskId: null,
        sessionId,
        workflowRunId: null,
      },
    }),
  selectSession: (sessionId, taskId, projectId) =>
    set({
      selection: {
        projectId,
        taskId,
        sessionId,
        workflowRunId: null,
      },
    }),
  selectWorkflowRun: (workflowRunId, projectId) =>
    set({
      selection: {
        projectId,
        taskId: null,
        sessionId: null,
        workflowRunId,
      },
    }),
  clearSelection: () => set({ selection: EMPTY_SELECTION }),
  clearSessionSelection: () =>
    set((state) => ({
      selection: {
        projectId: state.selection.projectId,
        taskId: state.selection.taskId,
        sessionId: null,
        workflowRunId: state.selection.workflowRunId,
      },
    })),
  clearTaskSelection: (projectId) =>
    set({
      selection: {
        projectId,
        taskId: null,
        sessionId: null,
        workflowRunId: null,
      },
    }),
  clearWorkflowRunSelection: (projectId) =>
    set({
      selection: {
        projectId,
        taskId: null,
        sessionId: null,
        workflowRunId: null,
      },
    }),
  setProject: (projectId) =>
    set({
      selection:
        projectId === null
          ? EMPTY_SELECTION
          : {
              projectId,
              taskId: null,
              sessionId: null,
              workflowRunId: null,
            },
    }),
}));
