import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { workspaceKeys } from "../data/workspace";
import { WORKSPACE_LIST_NOTIFY_PROPS } from "./workspace-list-query";

/** Loads visible Workspace identities used to resolve direct chats and workflow targets. */
export function useWorkspaces() {
  const client = useContractsClient();
  return useQuery({
    queryKey: workspaceKeys.workspaces,
    queryFn: () =>
      client.workspace.list({}).then((response) => response.workspaces),
    notifyOnChangeProps: [...WORKSPACE_LIST_NOTIFY_PROPS],
    // Each entry carries a probe of its folder, which users change outside Ora (deleting or
    // restoring a project in the file manager). Returning to the window is when that has
    // usually happened; structural sharing keeps subscribers asleep when nothing changed.
    refetchOnWindowFocus: true,
  });
}
