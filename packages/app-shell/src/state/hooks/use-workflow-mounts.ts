import { useQuery } from "@tanstack/react-query";
import { useWorkflowRuntime } from "../../features/workflow-run/use-workflow-runtime";
import { queryKeys } from "./query-keys";

/** Lists workflow definitions mounted on a project (D1: react-query list). */
export function useWorkflowMounts(projectId: string | null | undefined) {
  const runtime = useWorkflowRuntime();
  return useQuery({
    queryKey: queryKeys.workflowMounts(projectId ?? ""),
    queryFn: () => runtime.host.listMounts(projectId!),
    enabled: projectId != null && projectId !== "",
  });
}

/** Lists projects that already mount a given workflow definition. */
export function useWorkflowMountsByDefinition(
  definitionId: string | null | undefined,
) {
  const runtime = useWorkflowRuntime();
  return useQuery({
    queryKey: queryKeys.workflowMountsByDefinition(definitionId ?? ""),
    queryFn: () => runtime.host.listMountsByDefinition(definitionId!),
    enabled: definitionId != null && definitionId !== "",
  });
}
