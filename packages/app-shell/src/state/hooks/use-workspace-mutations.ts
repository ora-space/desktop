import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Project, Task } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import {
  cacheCreatedProject,
  cacheUpdatedProject,
  cacheDeletedProject,
  invalidateCreatedTask,
  cacheUpdatedTask,
  cacheDeletedTask,
  findSessionPlacement,
} from "../data/workspace";
import {
  cacheResumedSession,
  cacheDeletedSession,
  cacheRenamedSession,
  invalidateSessions,
  type WorkspaceListSync,
} from "../data/sessions";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useUiStore } from "../stores/ui-store";
import { useComposerInputStore } from "../stores/composer-input-store";
import { useDraftSessionsStore } from "../stores/draft-sessions-store";
import { startSessionDraft } from "../session-drafts";
import { useChatStore } from "../../chat-store-context";

/** Creates a project and selects it once the backend confirms the id. */
export function useCreateProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      name,
      mainWorkspacePath,
    }: {
      name: string;
      mainWorkspacePath: string;
    }) =>
      client.project
        .create({ name, mainWorkspacePath })
        .then((response) => response.project),
    onSuccess: (project) => {
      cacheCreatedProject(queryClient, project);
      startSessionDraft({ projectId: project.id, taskId: null });
    },
  });
}

/** Renames a project and patches the project list so the sidebar label updates immediately. */
export function useUpdateProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ project, name }: { project: Project; name: string }) =>
      client.project
        .update({ projectId: project.id, name })
        .then((response) => response.project),
    onSuccess: (project) => {
      cacheUpdatedProject(queryClient, project);
    },
  });
}

/** Deletes a project, cascading its tasks and sessions, then fixes the selection. */
export function useDeleteProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ projectId }: { projectId: string }) =>
      client.project.delete({ projectId }),
    onSuccess: (_void, { projectId }) => {
      const { taskIds, sessionIds, nextProjectId } = cacheDeletedProject(
        queryClient,
        projectId,
      );

      useComposerInputStore
        .getState()
        .clearKeys([
          ...sessionIds,
          ...[...taskIds].map((taskId) => `task:${taskId}`),
        ]);
      useDraftSessionsStore.getState().clearReturnToForSessions(sessionIds);
      useDraftSessionsStore.getState().removeForProject(projectId);
      const store = useWorkspaceSelectionStore.getState();
      const selection = store.selection;
      if (selection.projectId === projectId) {
        // setProject resyncs createFocus to the new selection. Preserve a
        // create-focus the user pointed at a different surviving project so New
        // chat still follows their last click, matching applyRestoredSelection.
        const focusBefore = store.createFocus;
        store.setProject(nextProjectId);
        if (focusBefore !== null && focusBefore.projectId !== projectId) {
          store.setCreateFocus(focusBefore);
        }
      } else {
        store.clearCreateFocusForProject(projectId);
      }
    },
  });
}

/** Creates a task under a project and selects it once the backend confirms the id. */
export function useCreateTask() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      projectId,
      title,
      baseBranch,
    }: {
      projectId: string;
      title: string;
      baseBranch?: string;
    }) =>
      client.task
        .create({ projectId, title, baseBranch })
        .then((response) => response.task),
    onSuccess: (task) => {
      invalidateCreatedTask(queryClient, task);
      startSessionDraft({ projectId: task.projectId, taskId: task.id });
      // Reveal the new row. Expanding here rather than reacting to the selection
      // keeps a plain row click free to collapse what it just selected.
      useUiStore.getState().expandProject(task.projectId);
    },
  });
}

/** Replaces a task's fields and patches the task list so the sidebar label updates immediately. */
export function useUpdateTask() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ task, title }: { task: Task; title: string }) =>
      client.task
        .update({ taskId: task.id, title })
        .then((response) => response.task),
    onSuccess: (task) => {
      cacheUpdatedTask(queryClient, task);
    },
  });
}

/** Deletes a task, cascading its sessions, and clears the task leg of the selection. */
export function useDeleteTask() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ taskId }: { taskId: string }) =>
      client.task.delete({ taskId }),
    onSuccess: ({ workspaceId }, { taskId }) => {
      const sessionIds = cacheDeletedTask(queryClient, taskId, workspaceId);

      useComposerInputStore
        .getState()
        .clearKeys([...sessionIds, `task:${taskId}`]);
      useDraftSessionsStore.getState().clearReturnToForSessions(sessionIds);
      useDraftSessionsStore.getState().removeForTask(taskId);
      const store = useWorkspaceSelectionStore.getState();
      const selection = store.selection;
      if (selection.taskId === taskId) {
        // clearTaskSelection resyncs createFocus to the project. Preserve a
        // create-focus the user pointed at a different surviving task so New
        // chat still follows their last click, matching applyRestoredSelection.
        const focusBefore = store.createFocus;
        store.clearTaskSelection(selection.projectId ?? "");
        if (focusBefore !== null && focusBefore.taskId !== taskId) {
          store.setCreateFocus(focusBefore);
        }
      } else {
        store.clearCreateFocusForTask(taskId);
      }
    },
  });
}

/**
 * Starts an additional provider session inside an existing Workspace and selects it.
 *
 * The model can still be changed from the composer once the session is selected.
 */
export function useCreateSession() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const chatStore = useChatStore();
  return useMutation({
    mutationFn: async ({
      workspaceId,
      agentCli,
    }: {
      workspaceId: string;
      agentCli: string;
    }) => {
      const response = await client.session.start({
        workspaceId,
        agentRef: agentCli,
        model: null,
      });
      chatStore
        .getState()
        .setConfigOptions(response.session.id, response.configOptions);
      return response.session;
    },
    onSuccess: (session) => {
      // A just-created provider session has no history to replay. Register an
      // empty loaded conversation so WorkspaceView does not issue session/load.
      chatStore.getState().initializeSession(session.id);
      void invalidateSessions(queryClient);
      // Recover project/task projection only for tree placement; persistence still
      // owns this session through its Workspace id.
      const { task, projectId } = findSessionPlacement(queryClient, session);
      if (projectId !== undefined) {
        if (task) {
          useWorkspaceSelectionStore
            .getState()
            .selectSession(session.id, task.id, projectId);
          useUiStore.getState().expandTask(task.id);
        } else {
          useWorkspaceSelectionStore
            .getState()
            .selectSessionBeforeTask(session.id, projectId);
        }
        useUiStore.getState().expandProject(projectId);
      }
    },
  });
}

/**
 * Returns a session whose history stopped being writable to a usable state.
 *
 * Everything else a degraded session can do is blocked until this succeeds —
 * prompting and switching agent both refuse — so the refreshed session is
 * pushed into the list rather than only invalidated, letting the surface that
 * offered the retry stop offering it without waiting for a refetch.
 */
export function useResumeSessionHistory() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) =>
      client.session
        .resumeHistory({ sessionId })
        .then((response) => response.session),
    onSuccess: (session) => {
      cacheResumedSession(queryClient, session);
    },
  });
}

/** Deletes a session and clears the session leg of the selection. */
export function useDeleteSession() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      sessionId,
    }: {
      sessionId: string;
      /** Parent project/task cascades pass `defer` to avoid N list refetches. */
      listSync?: WorkspaceListSync;
    }) => client.session.delete({ sessionId }),
    onSuccess: (_void, { sessionId, listSync = "immediate" }) => {
      cacheDeletedSession(queryClient, sessionId, listSync);
      useComposerInputStore.getState().clear(sessionId);
      useDraftSessionsStore.getState().clearReturnToForSessions([sessionId]);
      useDraftSessionsStore.getState().removeForSessions([sessionId]);
      const selection = useWorkspaceSelectionStore.getState().selection;
      if (selection.sessionId === sessionId) {
        useWorkspaceSelectionStore.getState().clearSessionSelection();
      }
    },
  });
}

/** Persists a user-edited session title and patches the sessions list cache. */
export function useRenameSession() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId, title }: { sessionId: string; title: string }) =>
      client.session
        .rename({ sessionId, title })
        .then((response) => response.session),
    onSuccess: (session) => {
      cacheRenamedSession(queryClient, session);
    },
  });
}
