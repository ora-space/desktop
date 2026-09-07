import type { WorkflowVariableCatalogEntry } from "@ora/workflow-mock";

/** One group in the unified variable list: a heading plus its variables. */
export interface WorkflowVariableMenuGroup {
  label: string;
  variables: WorkflowVariableCatalogEntry[];
}

/** Groups globals first, then each producing node, preserving catalog order within a group. */
export function groupWorkflowVariables(
  variables: WorkflowVariableCatalogEntry[],
  globalVariablesLabel: string,
): WorkflowVariableMenuGroup[] {
  const groups = new Map<string, WorkflowVariableMenuGroup>();
  for (const variable of variables) {
    const label =
      variable.scope === "global"
        ? globalVariablesLabel
        : (variable.sourceNodeTitle ?? variable.sourceNodeId);
    const group = groups.get(label);
    if (group !== undefined) {
      group.variables.push(variable);
    } else {
      groups.set(label, { label, variables: [variable] });
    }
  }
  return [...groups.values()];
}
