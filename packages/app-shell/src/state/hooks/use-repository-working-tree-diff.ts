import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads the main checkout patch for the repository Changes tab. */
export function useRepositoryWorkingTreeDiff(projectId: string | null) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.repositoryWorkingTreeDiff(projectId ?? ""),
    queryFn: () => client.repository
      .getWorkingTreeDiff({ projectId: projectId! })
      .then((response) => response.diff),
    enabled: projectId !== null,
  });
}
