import { useQuery } from "@tanstack/react-query";
import {
  type WorkspaceDiffScope,
  type WorkspaceDiffSelection,
  RemoteContractError,
} from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { diffKeys } from "../data/diff";

/** Loads the current Git snapshot for one workspace checkout — a task worktree or a project's
 * main checkout alike. */
export function useWorkspaceDiff(
  workspaceId: string,
  scope: WorkspaceDiffScope,
  enabled = true,
  selection?: WorkspaceDiffSelection,
  headCommitId?: string,
) {
  const client = useContractsClient();
  return useQuery({
    queryKey:
      selection === undefined
        ? diffKeys.workspaceDiff(workspaceId, scope)
        : diffKeys.workspaceDiffSelection(
            workspaceId,
            scope,
            selection,
            headCommitId,
          ),
    queryFn: () =>
      client.workspace.getDiff({
        workspaceId,
        scope,
        ...(selection === undefined ? {} : { selection }),
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
