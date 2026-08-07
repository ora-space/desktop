import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/**
 * Soft freshness window for branch lists.
 * Opening the picker still triggers a background refetch so newly pushed branches
 * appear without making the first paint wait on `git fetch`.
 */
const PROJECT_BRANCHES_STALE_MS = 60_000;

interface ProjectBranchesOptions {
  enabled?: boolean;
}

/** Loads refreshed local and remote refs that can seed a new worktree for the selected project. */
export function useProjectBranches(
  projectId: string | null,
  options: ProjectBranchesOptions = {},
) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.projectBranches(projectId ?? ""),
    queryFn: () => client.project
      .listBranches({ projectId: projectId! })
      .then((response) => response.branches),
    enabled: projectId !== null && (options.enabled ?? true),
    staleTime: PROJECT_BRANCHES_STALE_MS,
    gcTime: PROJECT_BRANCHES_STALE_MS * 10,
  });
}
