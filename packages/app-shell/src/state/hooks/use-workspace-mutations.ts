import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Project, Session, SessionStatus, Task, TaskStatus } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { useChatStore } from "../../chat-store-context";
import { queryKeys } from "./query-keys";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";

type QueryClient = ReturnType<typeof useQueryClient>;

/**
 * Agent a session starts with when nothing has been chosen explicitly.
 *
 * Shared by the session dialog and by the composer's implicit "first message
 * starts a session" path so the two cannot drift onto different agents.
 */
export const DEFAULT_AGENT_ID = "codex";

/** Reads the cached projects, tasks, or sessions, returning [] while data is absent. */
function readCache<T>(queryClient: QueryClient, key: readonly string[]): T[] {
  return (queryClient.getQueryData(key) as T[] | undefined) ?? [];
}

/** Creates a project and selects it once the server confirms the id. */
export function useCreateProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, rootPath }: { name: string; rootPath: string }) =>
      client.project.create({ name, rootPath }).then((response) => response.project),
    onSuccess: (project) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.projects });
      useWorkspaceSelectionStore.getState().selectProject(project.id);
    },
  });
}

/** Replaces a project's fields and refreshes the project list. */
export function useUpdateProject() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ project, name, rootPath }: { project: Project; name: string; rootPath: string }) =>
      client.project
        .update({ projectId: project.id, name, rootPath })
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
    mutationFn: async ({ projectId }: { projectId: string }) => {
      const tasks = readCache<Task>(queryClient, queryKeys.tasks);
      const sessions = readCache<Session>(queryClient, queryKeys.sessions);
      const childTasks = tasks.filter((task) => task.projectId === projectId);
      const childTaskIds = new Set(childTasks.map((task) => task.id));
      const childSessions = sessions.filter((session) => childTaskIds.has(session.taskId));
      await Promise.all(childSessions.map((session) => client.session.delete({ sessionId: session.id })));
      await Promise.all(childTasks.map((task) => client.task.delete({ taskId: task.id })));
      await client.project.delete({ projectId });
    },
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

/** Creates a task under a project and selects it once the server confirms the id. */
export function useCreateTask() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ projectId, title, status }: { projectId: string; title: string; status: TaskStatus }) =>
      client.task.create({ projectId, title, status }).then((response) => response.task),
    onSuccess: (task) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
      useWorkspaceSelectionStore.getState().selectTask(task.id, task.projectId);
    },
  });
}

/** Replaces a task's fields and refreshes the task list. */
export function useUpdateTask() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ task, title, status }: { task: Task; title: string; status: TaskStatus }) =>
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
    mutationFn: async ({ taskId }: { taskId: string }) => {
      const sessions = readCache<Session>(queryClient, queryKeys.sessions);
      const childSessions = sessions.filter((session) => session.taskId === taskId);
      await Promise.all(childSessions.map((session) => client.session.delete({ sessionId: session.id })));
      await client.task.delete({ taskId });
    },
    onSuccess: (_void, { taskId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      const selection = useWorkspaceSelectionStore.getState().selection;
      if (selection.taskId === taskId) {
        useWorkspaceSelectionStore.getState().clearTaskSelection(selection.projectId ?? "");
      }
    },
  });
}

/** Creates a session under a task and selects it once the server confirms the id. */
export function useCreateSession() {
  const client = useContractsClient();
  const chatStore = useChatStore();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({ taskId, agentId, status }: { taskId: string; agentId: string; status: SessionStatus }) => {
      const task = readCache<Task>(queryClient, queryKeys.tasks).find(
        (candidate) => candidate.id === taskId,
      );
      if (task === undefined) throw new Error(`task ${taskId} not found in workspace cache`);
      const project = readCache<Project>(queryClient, queryKeys.projects).find(
        (candidate) => candidate.id === task.projectId,
      );
      if (project === undefined) {
        throw new Error(`project ${task.projectId} not found in workspace cache`);
      }

      const agentSession = await chatStore.getState().newSession({
        cwd: project.rootPath,
        mcpServers: [],
      });
      return client.session
        .create({ taskId, agentId, agentSessionId: agentSession.sessionId, status })
        .then((response) => response.session);
    },
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
      // Recover the owning project from the task cache so selection stays consistent.
      const tasks = readCache<Task>(queryClient, queryKeys.tasks);
      const task = tasks.find((candidate) => candidate.id === session.taskId);
      if (task) {
        useWorkspaceSelectionStore.getState().selectSession(session.id, task.id, task.projectId);
      }
    },
  });
}

/** Replaces a session's fields and refreshes the session list. */
export function useUpdateSession() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ session, agentId, status }: { session: Session; agentId: string; status: SessionStatus }) =>
      client.session
        .update({
          sessionId: session.id,
          taskId: session.taskId,
          agentId,
          agentSessionId: session.agentSessionId,
          status,
        })
        .then((response) => response.session),
    onSuccess: () => {
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
