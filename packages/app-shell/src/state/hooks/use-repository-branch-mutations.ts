import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

type RepositoryBranchMutation = {
  projectId: string;
  branchName: string;
};

/** Creates a local branch and refreshes the repository refs used by the workspace. */
export function useCreateRepositoryBranch() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, branchName }: RepositoryBranchMutation) =>
      client.repository.createBranch({ projectId, branchName }),
    onSuccess: (_response, { projectId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.projectBranches(projectId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.repositorySnapshot(projectId) });
    },
  });
}

/** Checks out a clean main worktree and refreshes every repository view affected by HEAD. */
export function useCheckoutRepositoryBranch() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ projectId, branchName }: RepositoryBranchMutation) =>
      client.repository.checkoutBranch({ projectId, branchName }),
    onSuccess: (_response, { projectId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.projectBranches(projectId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.repositorySnapshot(projectId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.repositoryWorkingTreeDiff(projectId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.projectFiles(projectId) });
    },
  });
}
