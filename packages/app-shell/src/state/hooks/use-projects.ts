import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { workspaceKeys } from "../data/workspace";
import { WORKSPACE_LIST_NOTIFY_PROPS } from "./workspace-list-query";

/** Loads the visible project list through the contracts client and caches it. */
export function useProjects(options?: { enabled?: boolean }) {
  const client = useContractsClient();
  return useQuery({
    queryKey: workspaceKeys.projects,
    queryFn: () =>
      client.project.list({}).then((response) => response.projects),
    enabled: options?.enabled ?? true,
    notifyOnChangeProps: [...WORKSPACE_LIST_NOTIFY_PROPS],
  });
}
