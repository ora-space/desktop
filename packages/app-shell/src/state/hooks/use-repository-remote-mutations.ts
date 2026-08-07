import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { PullRepositoryStrategy, RepositorySyncAction } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

type RepositoryRemoteMutation = {
  projectId: string;
};

type RepositoryPullMutation = RepositoryRemoteMutation & {
  strategy: PullRepositoryStrategy;
};

type RepositorySyncMutation = RepositoryRemoteMutation & {
  action: RepositorySyncAction;
};

/** Refreshes repository graph, branch, and main-worktree queries after remote synchronization. */
function invalidateRepositoryRemoteQueries(
  queryClient: ReturnType<typeof useQueryClient>,
  projectId: string,
) {
  queryClient.invalidateQueries({ queryKey: queryKeys.repositorySnapshot(projectId) });
  queryClient.invalidateQueries({ queryKey: queryKeys.projectBranches(projectId) });
  queryClient.invalidateQueries({ queryKey: queryKeys.repositoryWorkingTreeDiff(projectId) });
}

/** Invalidates project file reads after a pull or sync changes the main checkout. */
function invalidateRepositoryFiles(
  queryClient: ReturnType<typeof useQueryClient>,
  projectId: string,
) {
  queryClient.invalidateQueries({ queryKey: queryKeys.projectFiles(projectId) });
}

/** Fetches all configured remotes and refreshes the repository graph. */
export function useFetchRepository() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId }: RepositoryRemoteMutation) =>
      client.repository.fetch({ projectId }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryRemoteQueries(queryClient, projectId);
    },
  });
}

/** Pulls the project's main branch with the caller-selected integration strategy. */
export function usePullRepository() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, strategy }: RepositoryPullMutation) =>
      client.repository.pull({ projectId, strategy }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryRemoteQueries(queryClient, projectId);
      invalidateRepositoryFiles(queryClient, projectId);
    },
  });
}

/** Continues or aborts the active merge/rebase and refreshes repository state. */
export function useResolveRepositorySync() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, action }: RepositorySyncMutation) =>
      client.repository.resolveSync({ projectId, action }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryRemoteQueries(queryClient, projectId);
      invalidateRepositoryFiles(queryClient, projectId);
    },
  });
}

/** Pushes the project's checked-out main branch to its default remote. */
export function usePushRepositoryBranch() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId }: RepositoryRemoteMutation) =>
      client.repository.pushBranch({ projectId }),
    onSuccess: (_response, { projectId }) => {
      invalidateRepositoryRemoteQueries(queryClient, projectId);
    },
  });
}
