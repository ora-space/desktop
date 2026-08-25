import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";
import { WORKSPACE_LIST_NOTIFY_PROPS } from "./workspace-list-query";

/** Loads visible Workspace identities used to resolve direct chats and workflow targets. */
export function useWorkspaces() {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.workspaces,
    queryFn: () =>
      client.workspace.list({}).then((response) => response.workspaces),
    notifyOnChangeProps: [...WORKSPACE_LIST_NOTIFY_PROPS],
  });
}
