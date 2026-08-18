import { useMutation, useQueryClient } from "@tanstack/react-query";
import type {
  AgentCli,
  Project,
  Session,
  Task,
  TaskStatus,
  TaskWorkspaceMode,
} from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useUiStore } from "../stores/ui-store";
import { useChatStore } from "../../chat-store-context";

type QueryClient = ReturnType<typeof useQueryClient>;

/** Reads the cached projects, tasks, or sessions, returning [] while data is absent. */
function readCache<T>(queryClient: QueryClient, key: readonly string[]): T[] {
  return (queryClient.getQueryData(key) as T[] | undefined) ?? [];
}

/** Creates a project and selects it once the backend confirms the id. */
export function useCreateProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, rootPath }: { name: string; rootPath: string }) =>
      client.project
        .create({ name, rootPath })
        .then((response) => response.project),
    onSuccess: (project) => {
      queryClient.setQueryData<Project[]>(queryKeys.projects, (current) => [
        ...(current ?? []).filter((candidate) => candidate.id !== project.id),
        project,
      ]);
      queryClient.invalidateQueries({ queryKey: queryKeys.projects });
      useWorkspaceSelectionStore.getState().selectProject(project.id);
    },
  });
}

/** Renames a project and refreshes the project list. */
export function useUpdateProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ project, name }: { project: Project; name: string }) =>
      client.project
        .update({ projectId: project.id, name })
        .then((response) => response.project),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.projects });
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
      queryClient.invalidateQueries({ queryKey: queryKeys.projects });
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      const selection = useWorkspaceSelectionStore.getState().selection;
      if (selection.projectId === projectId) {
        // Pick the next surviving project from the stale cache; invalidate already triggered refetch.
        const projects = readCache<Project>(queryClient, queryKeys.projects);
        const next = projects.find((project) => project.id !== projectId);
        useWorkspaceSelectionStore.getState().setProject(next?.id ?? null);
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
      status,
      workspaceMode,
      baseBranch,
    }: {
      projectId: string;
      title: string;
      status: TaskStatus;
      workspaceMode?: TaskWorkspaceMode;
      baseBranch?: string;
    }) =>
      client.task
        .create({ projectId, title, status, workspaceMode, baseBranch })
        .then((response) => response.task),
    onSuccess: (task) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
      // Worktrees preserve the original Task -> Session flow. A direct chat
      // waits until its provider session is ready before changing selection,
      // avoiding an intermediate task-only state in the composer.
      if (task.workspaceMode === "worktree") {
        // The backend created a new Ora branch, so the next worktree dialog
        // must refetch before offering base branches.
        queryClient.invalidateQueries({
          queryKey: queryKeys.projectBranches(task.projectId),
        });
        useWorkspaceSelectionStore
          .getState()
          .selectTask(task.id, task.projectId);
      }
      // Reveal the new row. Expanding here rather than reacting to the selection
      // keeps a plain row click free to collapse what it just selected.
      useUiStore.getState().expandProject(task.projectId);
    },
  });
}

/** Replaces a task's fields and refreshes the task list. */
export function useUpdateTask() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      task,
      title,
      status,
    }: {
      task: Task;
      title: string;
      status: TaskStatus;
    }) =>
      client.task
        .update({ taskId: task.id, title, status })
        .then((response) => response.task),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
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
    onSuccess: (_void, { taskId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      const selection = useWorkspaceSelectionStore.getState().selection;
      if (selection.taskId === taskId) {
        useWorkspaceSelectionStore
          .getState()
          .clearTaskSelection(selection.projectId ?? "");
      }
    },
  });
}

/**
 * Starts an additional session under an existing task and selects it.
 *
 * A provider session is warmed and then persisted in one step because there is
 * no chat surface here to warm it in advance; the model can still be changed
 * from the composer once the session is selected.
 */
export function useCreateSession() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const chatStore = useChatStore();
  return useMutation({
    mutationFn: async ({
      taskId,
      agentCli,
    }: {
      taskId: string;
      agentCli: AgentCli;
    }) => {
      const warmed = await client.session.warm({
        target: { type: "task", taskId },
        agentCli,
      });
      const response = await client.session.attach({
        sessionId: warmed.sessionId,
        taskId,
      });
      queryClient.removeQueries({
        queryKey: queryKeys.warmSession({ type: "task", taskId }, agentCli),
      });
      chatStore
        .getState()
        .setConfigOptions(response.session.id, warmed.configOptions);
      return response.session;
    },
    onSuccess: (session) => {
      // A just-created provider session has no history to replay. Register an
      // empty loaded conversation so WorkspaceView does not issue session/load.
      chatStore.getState().initializeSession(session.id);
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      // Recover the owning project from the task cache so selection stays consistent.
      const tasks = readCache<Task>(queryClient, queryKeys.tasks);
      const task = tasks.find((candidate) => candidate.id === session.taskId);
      if (task) {
        useWorkspaceSelectionStore
          .getState()
          .selectSession(session.id, task.id, task.projectId);
        // Both ancestors, since the session sits two levels down.
        useUiStore.getState().expandProject(task.projectId);
        useUiStore.getState().expandTask(task.id);
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
      queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
        (current ?? []).map((candidate) =>
          candidate.id === session.id ? session : candidate,
        ),
      );
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
    },
  });
}

/** Deletes a session and clears the session leg of the selection. */
export function useDeleteSession() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ sessionId }: { sessionId: string }) =>
      client.session.delete({ sessionId }),
    onSuccess: (_void, { sessionId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      const selection = useWorkspaceSelectionStore.getState().selection;
      if (selection.sessionId === sessionId) {
        useWorkspaceSelectionStore.getState().clearSessionSelection();
      }
    },
  });
}
