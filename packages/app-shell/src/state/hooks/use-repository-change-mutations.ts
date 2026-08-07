import type { RepositoryChangeSelection, RepositoryConflictSide } from "@ora/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

type RepositoryChangeMutation = {
  projectId: string;
  selection: RepositoryChangeSelection;
};

type RepositoryCommitMutation = {
  projectId: string;
  message: string;
};

type RepositoryConflictMutation = {
  projectId: string;
  path: string;
  side: RepositoryConflictSide;
};

/** Refreshes both the status counters and patch after a repository mutation. */
function invalidateRepositoryChanges(queryClient: ReturnType<typeof useQueryClient>, projectId: string) {
  queryClient.invalidateQueries({ queryKey: queryKeys.repositorySnapshot(projectId) });
  queryClient.invalidateQueries({ queryKey: queryKeys.repositoryWorkingTreeDiff(projectId) });
}

/** Invalidates project file reads after a mutation changes checkout contents. */
function invalidateRepositoryFiles(
  queryClient: ReturnType<typeof useQueryClient>,
  projectId: string,
) {
  queryClient.invalidateQueries({ queryKey: queryKeys.projectFiles(projectId) });
}

/** Stages selected paths in the project's main repository worktree. */
export function useStageRepositoryChanges() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, selection }: RepositoryChangeMutation) =>
      client.repository.stageChanges({ projectId, selection }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryChanges(queryClient, projectId);
    },
  });
}

/** Removes selected paths from the project's main repository index. */
export function useUnstageRepositoryChanges() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, selection }: RepositoryChangeMutation) =>
      client.repository.unstageChanges({ projectId, selection }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryChanges(queryClient, projectId);
    },
  });
}

/** Selects and stages one side of a conflicted path in the project's main worktree. */
export function useResolveRepositoryConflict() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, path, side }: RepositoryConflictMutation) =>
      client.repository.resolveConflict({ projectId, path, side }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryChanges(queryClient, projectId);
      invalidateRepositoryFiles(queryClient, projectId);
    },
  });
}

/** Commits the currently staged changes in the project's main repository worktree. */
export function useCommitRepositoryChanges() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, message }: RepositoryCommitMutation) =>
      client.repository.commitChanges({ projectId, message }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryChanges(queryClient, projectId);
      invalidateRepositoryFiles(queryClient, projectId);
    },
  });
}
