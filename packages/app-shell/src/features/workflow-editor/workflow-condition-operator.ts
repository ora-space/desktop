import type { WorkflowVariableValueType } from "@ora/workflow-mock";

/** Offers comparisons appropriate for the selected value, preserving unknown-type editing. */
export function operatorSupportsType(
  operator: string,
  valueType: WorkflowVariableValueType | undefined,
): boolean {
  if (valueType === undefined || valueType === "any") return true;
  switch (operator) {
    case "greater_than":
    case "greater_than_or_equal":
    case "less_than":
    case "less_than_or_equal":
      return valueType === "number" || valueType === "integer";
    case "contains":
    case "not_contains":
      return (
        valueType === "string" ||
        valueType === "secret" ||
        valueType.startsWith("array")
      );
    case "starts_with":
    case "ends_with":
      return valueType === "string" || valueType === "secret";
    default:
      return true;
  }
}
