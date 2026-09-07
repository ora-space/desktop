import type { QueryClient } from "@tanstack/react-query";
import type {
  ContractsClient,
  WorkspaceFileChange,
  WorkspaceSearchKind,
} from "@ora/contracts";

/** Selects task worktree APIs when a task exists; otherwise the project checkout. */
export type FilesScope =
  { kind: "task"; taskId: string } | { kind: "project"; projectId: string };

/** Resolves whether Files should browse a task worktree or the project checkout. */
export function resolveFilesScope(
  projectId: string,
  taskId: string | undefined,
): FilesScope {
  return taskId !== undefined
    ? { kind: "task", taskId }
    : { kind: "project", projectId };
}

/** Builds the react-query key for one file preview in the active Files scope. */
export function fileQueryKey(scope: FilesScope, path: string) {
  return scope.kind === "task"
    ? fileKeys.workspaceFile(scope.taskId, path)
    : fileKeys.projectFile(scope.projectId, path);
}

/** Builds the react-query key for one search/filter query in the active Files scope. */
export function searchQueryKey(
  scope: FilesScope,
  kind: WorkspaceSearchKind | "files",
  query: string,
) {
  return scope.kind === "task"
    ? fileKeys.workspaceSearch(scope.taskId, kind, query)
    : fileKeys.projectSearch(scope.projectId, kind, query);
}

/** Prefix that invalidates every directory/file/search query for one Files scope. */
export function filesScopeQueryKey(scope: FilesScope) {
  return scope.kind === "task"
    ? fileKeys.workspaceFiles(scope.taskId)
    : fileKeys.projectFiles(scope.projectId);
}

/** Directory listing key for one expanded path in the active Files scope. */
export function directoryQueryKey(scope: FilesScope, path: string) {
  return scope.kind === "task"
    ? fileKeys.workspaceDirectory(scope.taskId, path)
    : fileKeys.projectDirectory(scope.projectId, path);
}

/** Thin client adapter so list/search/read/watch share one scope branch. */
export function filesScopeApi(client: ContractsClient, scope: FilesScope) {
  return {
    listDirectory(path: string, signal?: AbortSignal) {
      return scope.kind === "task"
        ? client.fileSystem.listWorkspaceDirectory(
            {
              taskId: scope.taskId,
              ...(path === "" ? {} : { path }),
            },
            { signal },
          )
        : client.fileSystem.listProjectDirectory(
            {
              projectId: scope.projectId,
              ...(path === "" ? {} : { path }),
            },
            { signal },
          );
    },
    readFile(path: string, signal?: AbortSignal) {
      return scope.kind === "task"
        ? client.fileSystem.readWorkspaceFile(
            { taskId: scope.taskId, path },
            { signal },
          )
        : client.fileSystem.readProjectFile(
            { projectId: scope.projectId, path },
            { signal },
          );
    },
    search(query: string, kind: WorkspaceSearchKind, signal?: AbortSignal) {
      return scope.kind === "task"
        ? client.fileSystem.searchWorkspace(
            { taskId: scope.taskId, query, kind },
            { signal },
          )
        : client.fileSystem.searchProject(
            { projectId: scope.projectId, query, kind },
            { signal },
          );
    },
    watch(signal?: AbortSignal) {
      return scope.kind === "task"
        ? client.fileSystem.watchWorkspace({ taskId: scope.taskId }, { signal })
        : client.fileSystem.watchProject(
            { projectId: scope.projectId },
            { signal },
          );
    },
  };
}

/** Invalidates only the scoped file queries affected by one native event batch. */
export async function invalidateScopedFileQueries(
  queryClient: QueryClient,
  scope: FilesScope,
  changes: WorkspaceFileChange[],
): Promise<void> {
  const directoryPaths = new Set<string>();
  const filePaths = new Set<string>();
  let invalidateSearch = false;
  let invalidateAll = false;

  for (const change of changes) {
    if (change.kind === "rescanRequired") {
      invalidateAll = true;
      break;
    }
    invalidateSearch = true;
    filePaths.add(change.path);
    directoryPaths.add(parentPath(change.path));
    if (change.kind === "renamed") {
      filePaths.add(change.from);
      directoryPaths.add(parentPath(change.from));
    }
  }

  if (invalidateAll) {
    await queryClient.invalidateQueries({
      queryKey: filesScopeQueryKey(scope),
    });
    return;
  }

  await Promise.all([
    ...Array.from(directoryPaths, (path) =>
      queryClient.invalidateQueries({
        queryKey: directoryQueryKey(scope, path),
      }),
    ),
    ...Array.from(filePaths, (path) =>
      queryClient.invalidateQueries({
        queryKey: fileQueryKey(scope, path),
      }),
    ),
    ...(invalidateSearch
      ? [
          queryClient.invalidateQueries({
            queryKey: [...filesScopeQueryKey(scope), "search"],
          }),
        ]
      : []),
  ]);
}

/** Returns the parent directory for a normalized workspace-relative path. */
export function parentPath(path: string): string {
  const separator = path.lastIndexOf("/");
  return separator <= 0 ? "" : path.slice(0, separator);
}

/** Cache identity owned by files data; consumers never repeat its tuples. */
export const fileKeys = {
  workspaceFiles: (taskId: string) => ["workspace-files", taskId] as const,
  workspaceDirectory: (taskId: string, path: string) =>
    ["workspace-files", taskId, "directory", path] as const,
  workspaceFile: (taskId: string, path: string) =>
    ["workspace-files", taskId, "file", path] as const,
  workspaceSearch: (taskId: string, kind: string, query: string) =>
    ["workspace-files", taskId, "search", kind, query] as const,
  projectFiles: (projectId: string) => ["project-files", projectId] as const,
  projectDirectory: (projectId: string, path: string) =>
    ["project-files", projectId, "directory", path] as const,
  projectFile: (projectId: string, path: string) =>
    ["project-files", projectId, "file", path] as const,
  projectSearch: (projectId: string, kind: string, query: string) =>
    ["project-files", projectId, "search", kind, query] as const,
};

/** Refreshes directory, file, and search projections for exactly one Files scope. */
export function invalidateFilesScope(
  queryClient: QueryClient,
  scope: FilesScope,
) {
  return queryClient.invalidateQueries({ queryKey: filesScopeQueryKey(scope) });
}
