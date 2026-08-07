import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads one commit's changed paths only after a graph row has been selected. */
export function useRepositoryCommit(projectId: string, commitId: string | null) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.repositoryCommit(projectId, commitId ?? ""),
    queryFn: () => client.repository
      .getCommit({ projectId, commitId: commitId! })
      .then((response) => response.commit),
    enabled: commitId !== null,
  });
}
