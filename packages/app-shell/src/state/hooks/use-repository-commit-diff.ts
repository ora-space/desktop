import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

interface RepositoryCommitDiffOptions {
  enabled: boolean;
}

/** Loads a historical commit patch only while the selected file dialog is open. */
export function useRepositoryCommitDiff(
  projectId: string,
  commitId: string | null,
  parentCommitId: string | null,
  path: string | null,
  { enabled }: RepositoryCommitDiffOptions,
) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.repositoryCommitDiff(projectId, commitId ?? "", path ?? ""),
    queryFn: () => client.repository
      .getCommitDiff({
        projectId,
        commitId: commitId!,
        parentCommitId,
        path: path!,
      })
      .then((response) => response.patch),
    enabled: enabled && commitId !== null && path !== null,
  });
}
