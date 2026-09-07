import type { WorkflowVariableCatalogEntry } from "@ora/workflow-mock";

/** Formats a variable using only its stable fully qualified selector. */
export function workflowVariableLabel(
  variable: WorkflowVariableCatalogEntry,
): string {
  return variable.selector.join(".");
}

/** Formats the compact blue token rendered inside a workflow prompt. */
export function workflowVariableTokenLabel(
  variable: WorkflowVariableCatalogEntry,
): string {
  return variable.selector.join(".");
}
