import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { workspaceKeys } from "../data/workspace";
import { WORKSPACE_LIST_NOTIFY_PROPS } from "./workspace-list-query";

/** Loads the visible task list through the contracts client and caches it. */
export function useTasks() {
  const client = useContractsClient();
  return useQuery({
    queryKey: workspaceKeys.tasks,
    queryFn: () => client.task.list({}).then((response) => response.tasks),
    notifyOnChangeProps: [...WORKSPACE_LIST_NOTIFY_PROPS],
  });
}
