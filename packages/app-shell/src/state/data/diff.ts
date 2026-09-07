import type { QueryClient } from "@tanstack/react-query";

import type { WorkspaceDiffScope } from "@ora/contracts";

/** Cache identity owned by diff data; consumers never repeat its tuples. */
export const diffKeys = {
  workspaceDiffs: (workspaceId: string) =>
    ["workspace-diff", workspaceId] as const,
  workspaceDiff: (workspaceId: string, scope: WorkspaceDiffScope) =>
    ["workspace-diff", workspaceId, scope] as const,
};

/** Refreshes every diff scope for this workspace, without affecting another worktree. */
export function invalidateWorkspaceDiffs(
  queryClient: QueryClient,
  workspaceId: string,
) {
  return queryClient.invalidateQueries({
    queryKey: diffKeys.workspaceDiffs(workspaceId),
  });
}
