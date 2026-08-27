import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Project, Session, Task, Workspace } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useUiStore } from "../stores/ui-store";
import { useComposerInputStore } from "../stores/composer-input-store";
import { useDraftSessionsStore } from "../stores/draft-sessions-store";
import { startSessionDraft } from "../session-drafts";
import { useChatStore } from "../../chat-store-context";

type QueryClient = ReturnType<typeof useQueryClient>;

/**
 * How list caches should sync after a mutation.
 *
 * - `immediate`: patch and mark the list stale with an active refetch — use for
 *   standalone deletes the user just confirmed.
 * - `defer`: patch only; a parent cascade will invalidate once at the end so
 *   N child deletes do not thrash the sidebar N times.
 */
export type WorkspaceListSync = "immediate" | "defer";

/** Reads the cached projects, tasks, or sessions, returning [] while data is absent. */
function readCache<T>(queryClient: QueryClient, key: readonly string[]): T[] {
  return (queryClient.getQueryData(key) as T[] | undefined) ?? [];
}

/** Marks list queries stale; `none` skips the active refetch that rebuilds every subscriber. */
function invalidateWorkspaceLists(
  queryClient: QueryClient,
  keys: readonly (readonly string[])[],
  refetchType: "active" | "none" = "active",
): void {
  for (const queryKey of keys) {
    void queryClient.invalidateQueries({ queryKey, refetchType });
  }
}

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
      queryClient.setQueryData<Project[]>(queryKeys.projects, (current) => [
        ...(current ?? []).filter((candidate) => candidate.id !== project.id),
        project,
      ]);
      queryClient.invalidateQueries({ queryKey: queryKeys.projects });
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces });
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
      queryClient.setQueryData<Project[]>(queryKeys.projects, (current) =>
        (current ?? []).map((candidate) =>
          candidate.id === project.id ? { ...project } : candidate,
        ),
      );
      // Response already applied; mark stale without refetching every subscriber.
      invalidateWorkspaceLists(
        queryClient,
        [queryKeys.projects],
        /*refetchType*/ "none",
      );
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
      const tasks = readCache<Task>(queryClient, queryKeys.tasks);
      const taskIds = new Set(
        tasks
          .filter((task) => task.projectId === projectId)
          .map((task) => task.id),
      );
      const workspaces = readCache<Workspace>(
        queryClient,
        queryKeys.workspaces,
      );
      const workspaceIds = new Set([
        ...workspaces
          .filter((workspace) => workspace.projectId === projectId)
          .map((workspace) => workspace.id),
        ...tasks
          .filter((task) => task.projectId === projectId)
          .map((task) => task.workspaceId),
      ]);
      const sessions = readCache<Session>(queryClient, queryKeys.sessions);
      const sessionIds = sessions
        .filter((session) => workspaceIds.has(session.workspaceId))
        .map((session) => session.id);

      // Optimistic scrub so the sidebar drops the branch before refetch settles.
      queryClient.setQueryData<Project[]>(queryKeys.projects, (current) =>
        (current ?? []).filter((project) => project.id !== projectId),
      );
      queryClient.setQueryData<Task[]>(queryKeys.tasks, (current) =>
        (current ?? []).filter((task) => task.projectId !== projectId),
      );
      queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
        (current ?? []).filter(
          (session) => !workspaceIds.has(session.workspaceId),
        ),
      );
      invalidateWorkspaceLists(queryClient, [
        queryKeys.projects,
        queryKeys.workspaces,
        queryKeys.tasks,
        queryKeys.sessions,
      ]);

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
        const projects = readCache<Project>(queryClient, queryKeys.projects);
        const next = projects.find((project) => project.id !== projectId);
        // setProject resyncs createFocus to the new selection. Preserve a
        // create-focus the user pointed at a different surviving project so New
        // chat still follows their last click, matching applyRestoredSelection.
        const focusBefore = store.createFocus;
        store.setProject(next?.id ?? null);
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
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks });
      // The backend created a new Ora branch, so the next worktree dialog must
      // refetch before offering base branches.
      queryClient.invalidateQueries({
        queryKey: queryKeys.projectBranches(task.projectId),
      });
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
      queryClient.setQueryData<Task[]>(queryKeys.tasks, (current) =>
        (current ?? []).map((candidate) =>
          candidate.id === task.id ? { ...task } : candidate,
        ),
      );
      invalidateWorkspaceLists(
        queryClient,
        [queryKeys.tasks],
        /*refetchType*/ "none",
      );
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
      const sessions = readCache<Session>(queryClient, queryKeys.sessions);
      const sessionIds = sessions
        .filter((session) => session.workspaceId === workspaceId)
        .map((session) => session.id);

      queryClient.setQueryData<Task[]>(queryKeys.tasks, (current) =>
        (current ?? []).filter((task) => task.id !== taskId),
      );
      queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
        (current ?? []).filter(
          (session) => session.workspaceId !== workspaceId,
        ),
      );
      invalidateWorkspaceLists(queryClient, [
        queryKeys.workspaces,
        queryKeys.tasks,
        queryKeys.sessions,
      ]);

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
      workspaceId,
      agentCli,
    }: {
      workspaceId: string;
      agentCli: string;
    }) => {
      const warmed = await client.session.warm({
        target: { type: "workspace", workspaceId },
        agentRef: agentCli,
      });
      const response = await client.session.attach({
        sessionId: warmed.sessionId,
        workspaceId: warmed.workspaceId,
      });
      queryClient.removeQueries({
        queryKey: queryKeys.warmSession(
          { type: "workspace", workspaceId: warmed.workspaceId },
          agentCli,
        ),
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
      // Recover project/task projection only for tree placement; persistence still
      // owns this session through its Workspace id.
      const tasks = readCache<Task>(queryClient, queryKeys.tasks);
      const task = tasks.find(
        (candidate) => candidate.workspaceId === session.workspaceId,
      );
      const workspace = readCache<Workspace>(
        queryClient,
        queryKeys.workspaces,
      ).find((candidate) => candidate.id === session.workspaceId);
      const projectId = workspace?.projectId ?? task?.projectId;
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
    mutationFn: ({
      sessionId,
    }: {
      sessionId: string;
      /** Parent project/task cascades pass `defer` to avoid N list refetches. */
      listSync?: WorkspaceListSync;
    }) => client.session.delete({ sessionId }),
    onSuccess: (_void, { sessionId, listSync = "immediate" }) => {
      queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
        (current ?? []).filter((session) => session.id !== sessionId),
      );
      if (listSync === "immediate") {
        invalidateWorkspaceLists(queryClient, [queryKeys.sessions]);
      }
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
      // Replace with a new object so React Query cannot structural-share the
      // previous cache entry when the transport returns the same session reference.
      queryClient.setQueryData<Session[]>(queryKeys.sessions, (current) =>
        (current ?? []).map((candidate) =>
          candidate.id === session.id ? { ...session } : candidate,
        ),
      );
      invalidateWorkspaceLists(
        queryClient,
        [queryKeys.sessions],
        /*refetchType*/ "none",
      );
    },
  });
}
