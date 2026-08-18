import { useQuery } from "@tanstack/react-query";
import { useOptionalContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Resolves the active backend-managed checkout for one selected task. */
export function useTaskWorkspace(taskId: string | undefined) {
  const client = useOptionalContractsClient();
  return useQuery({
    queryKey: queryKeys.taskWorkspace(taskId ?? ""),
    queryFn: () => {
      if (client === null) {
        return Promise.reject(new Error("ContractsClient not available"));
      }
      return client.task
        .getWorkspace({ taskId: taskId! })
        .then((response) => response.workspace);
    },
    enabled: taskId !== undefined && client !== null,
  });
}
