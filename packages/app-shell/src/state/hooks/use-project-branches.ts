import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads refreshed local and remote refs that can seed a new worktree for the selected project. */
export function useProjectBranches(projectId: string | null) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.projectBranches(projectId ?? ""),
    queryFn: () => client.project
      .listBranches({ projectId: projectId! })
      .then((response) => response.branches),
    enabled: projectId !== null,
  });
}
