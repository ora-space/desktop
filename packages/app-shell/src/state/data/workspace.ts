import type { Project, Session, Task, Workspace } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";
import { sessionKeys } from "./sessions";

/** Cache identity owned by workspace data; consumers never repeat its tuples. */
export const workspaceKeys = {
  projects: ["projects"] as const,
  workspaces: ["workspaces"] as const,
  projectBranches: (projectId: string) =>
    ["project-branches", projectId] as const,
  tasks: ["tasks"] as const,
  taskWorkspace: (taskId: string) => ["task-workspace", taskId] as const,
  workspaceCwd: (workspaceId: string) =>
    ["workspace-cwd", workspaceId] as const,
};

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

/** Adds the confirmed project and refreshes its workspace projection. */
export function cacheCreatedProject(
  queryClient: QueryClient,
  project: Project,
) {
  queryClient.setQueryData<Project[]>(workspaceKeys.projects, (current) => [
    ...(current ?? []).filter((candidate) => candidate.id !== project.id),
    project,
  ]);
  queryClient.invalidateQueries({ queryKey: workspaceKeys.projects });
  queryClient.invalidateQueries({ queryKey: workspaceKeys.workspaces });
}

/** Applies a rename immediately without refetching every sidebar subscriber. */
export function cacheUpdatedProject(
  queryClient: QueryClient,
  project: Project,
) {
  queryClient.setQueryData<Project[]>(workspaceKeys.projects, (current) =>
    (current ?? []).map((candidate) =>
      candidate.id === project.id ? { ...project } : candidate,
    ),
  );
  // Response already applied; mark stale without refetching every subscriber.
  invalidateWorkspaceLists(
    queryClient,
    [workspaceKeys.projects],
    /*refetchType*/ "none",
  );
}

/** Scrubs a deleted aggregate and returns affected UI identities before refetch settles. */
export function cacheDeletedProject(
  queryClient: QueryClient,
  projectId: string,
) {
  const tasks = readCache<Task>(queryClient, workspaceKeys.tasks);
  const taskIds = new Set(
    tasks.filter((task) => task.projectId === projectId).map((task) => task.id),
  );
  const workspaces = readCache<Workspace>(
    queryClient,
    workspaceKeys.workspaces,
  );
  const workspaceIds = new Set([
    ...workspaces
      .filter((workspace) => workspace.projectId === projectId)
      .map((workspace) => workspace.id),
    ...tasks
      .filter((task) => task.projectId === projectId)
      .map((task) => task.workspaceId),
  ]);
  const sessions = readCache<Session>(queryClient, sessionKeys.sessions);
  const sessionIds = sessions
    .filter((session) => workspaceIds.has(session.workspaceId))
    .map((session) => session.id);

  // Optimistic scrub so the sidebar drops the branch before refetch settles.
  queryClient.setQueryData<Project[]>(workspaceKeys.projects, (current) =>
    (current ?? []).filter((project) => project.id !== projectId),
  );
  queryClient.setQueryData<Task[]>(workspaceKeys.tasks, (current) =>
    (current ?? []).filter((task) => task.projectId !== projectId),
  );
  queryClient.setQueryData<Session[]>(sessionKeys.sessions, (current) =>
    (current ?? []).filter((session) => !workspaceIds.has(session.workspaceId)),
  );
  invalidateWorkspaceLists(queryClient, [
    workspaceKeys.projects,
    workspaceKeys.workspaces,
    workspaceKeys.tasks,
    sessionKeys.sessions,
  ]);

  return {
    taskIds,
    sessionIds,
    nextProjectId:
      readCache<Project>(queryClient, workspaceKeys.projects).find(
        (project) => project.id !== projectId,
      )?.id ?? null,
  };
}

/** Refreshes task and branch lists after the backend creates an Ora branch. */
export function invalidateCreatedTask(queryClient: QueryClient, task: Task) {
  queryClient.invalidateQueries({ queryKey: workspaceKeys.tasks });
  // The backend created a new Ora branch, so the next worktree dialog must
  // refetch before offering base branches.
  queryClient.invalidateQueries({
    queryKey: workspaceKeys.projectBranches(task.projectId),
  });
}

/** Applies the returned task and marks the list stale without an active refetch. */
export function cacheUpdatedTask(queryClient: QueryClient, task: Task) {
  queryClient.setQueryData<Task[]>(workspaceKeys.tasks, (current) =>
    (current ?? []).map((candidate) =>
      candidate.id === task.id ? { ...task } : candidate,
    ),
  );
  invalidateWorkspaceLists(
    queryClient,
    [workspaceKeys.tasks],
    /*refetchType*/ "none",
  );
}

/** Scrubs a task and its workspace sessions, returning ids for UI cleanup. */
export function cacheDeletedTask(
  queryClient: QueryClient,
  taskId: string,
  workspaceId: string,
) {
  const sessions = readCache<Session>(queryClient, sessionKeys.sessions);
  const sessionIds = sessions
    .filter((session) => session.workspaceId === workspaceId)
    .map((session) => session.id);

  queryClient.setQueryData<Task[]>(workspaceKeys.tasks, (current) =>
    (current ?? []).filter((task) => task.id !== taskId),
  );
  queryClient.setQueryData<Session[]>(sessionKeys.sessions, (current) =>
    (current ?? []).filter((session) => session.workspaceId !== workspaceId),
  );
  invalidateWorkspaceLists(queryClient, [
    workspaceKeys.workspaces,
    workspaceKeys.tasks,
    sessionKeys.sessions,
  ]);

  return sessionIds;
}

/** Projects workspace ownership onto the sidebar tree; it does not change persistence. */
export function findSessionPlacement(
  queryClient: QueryClient,
  session: Session,
) {
  const tasks = readCache<Task>(queryClient, workspaceKeys.tasks);
  const task = tasks.find(
    (candidate) => candidate.workspaceId === session.workspaceId,
  );
  const workspace = readCache<Workspace>(
    queryClient,
    workspaceKeys.workspaces,
  ).find((candidate) => candidate.id === session.workspaceId);
  const projectId = workspace?.projectId ?? task?.projectId;
  return { task, projectId };
}
