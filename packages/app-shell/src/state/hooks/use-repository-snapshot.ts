import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads the bounded Git snapshot used by the repository graph surface. */
export function useRepositorySnapshot(projectId: string | null) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.repositorySnapshot(projectId ?? ""),
    queryFn: () => client.repository
      .getSnapshot({ projectId: projectId! })
      .then((response) => response.snapshot),
    enabled: projectId !== null,
  });
}
