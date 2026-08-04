import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Resolves the active backend-managed checkout for one selected task. */
export function useTaskWorkspace(taskId: string | undefined) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.taskWorkspace(taskId ?? ""),
    queryFn: () => client.task.getWorkspace({ taskId: taskId! }).then((response) => response.workspace),
    enabled: taskId !== undefined,
  });
}
