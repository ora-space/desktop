import { resolveConditionCases } from "@ora/workflow-mock";
import type { WorkflowNodeData } from "@ora/workflow-runtime";

/** Returns every persisted branch port a Condition exposes, including its fallback edge. */
export function resolveRunOverviewSourceHandleIds(
  data: WorkflowNodeData,
): string[] | null {
  if (data.kind !== "condition") return null;
  return [
    ...resolveConditionCases(data).map((conditionCase) => conditionCase.id),
    "else",
  ];
}
