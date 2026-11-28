import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  RemoteContractError,
  type WorkspaceDiffScope,
  type WorkspaceDiffSelection,
} from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { diffKeys } from "../data/diff";

/** Loads the current Git snapshot for one workspace checkout — a task worktree or a project's
 * main checkout alike. */
export function useWorkspaceDiff(
  workspaceId: string,
  scope: WorkspaceDiffScope,
  enabled = true,
  selection: WorkspaceDiffSelection = { kind: "patch" },
  headCommitId?: string,
) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const queryKey =
    selection.kind === "patch"
      ? diffKeys.workspaceDiff(workspaceId, scope)
      : diffKeys.workspaceDiffSelection(
          workspaceId,
          scope,
          selection,
          headCommitId,
        );
  const retry =
    queryClient.getQueryDefaults(queryKey).retry ??
    queryClient.getDefaultOptions().queries?.retry ??
    3;
  return useQuery({
    queryKey,
    retry: (failureCount, error) => {
      if (isWorkspaceDiffTooLarge(error)) return false;
      if (typeof retry === "function") return retry(failureCount, error);
      return typeof retry === "number" ? failureCount < retry : retry;
    },
    queryFn: () =>
      client.workspace.getDiff({
        workspaceId,
        scope,
        selection,
      }),
    enabled: enabled && workspaceId !== "",
  });
}

/** Size-limit failures need a different read, not an identical retry. */
export function isWorkspaceDiffTooLarge(error: unknown): boolean {
  return (
    error instanceof RemoteContractError &&
    error.code === "workspace_diff_too_large"
  );
}
