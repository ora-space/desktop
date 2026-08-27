import { useQuery } from "@tanstack/react-query";
import type { WorkspaceDiffScope } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads the current Git snapshot for one workspace checkout — a task worktree or a project's
 * main checkout alike. */
export function useWorkspaceDiff(
  workspaceId: string,
  scope: WorkspaceDiffScope,
  enabled = true,
) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.workspaceDiff(workspaceId, scope),
    queryFn: () => client.workspace.getDiff({ workspaceId, scope }),
    enabled: enabled && workspaceId !== "",
  });
}
