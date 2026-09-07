import { SelectGroup, SelectItem, SelectLabel } from "@ora/ui";
import type { WorkflowVariableCatalogEntry } from "@ora/workflow-mock";
import { groupWorkflowVariables } from "./workflow-variable-groups";

/** Renders one variable row with stable semantic colors across hover and focus states. */
export function WorkflowVariableRowContent({
  variable,
}: {
  variable: WorkflowVariableCatalogEntry;
}) {
  return (
    <span className="flex w-full min-w-0 items-center justify-between gap-3">
      <span className="flex min-w-0 items-center gap-1.5">
        <span
          aria-hidden="true"
          data-workflow-variable-part="variable-mark"
          className="shrink-0 font-medium text-blue-600 dark:text-blue-400"
        >
          {"{x}"}
        </span>
        <span
          data-workflow-variable-part="variable-name"
          className="truncate font-medium text-foreground"
        >
          {variable.variableName}
        </span>
      </span>
      <span
        data-workflow-variable-part="variable-type"
        className="shrink-0 text-[11px] capitalize text-muted-foreground"
      >
        {variable.valueType}
      </span>
    </span>
  );
}

/** Renders a shadcn Select popover's structured groups for a node's selectable variables. */
export function WorkflowVariableSelectGroups({
  variables,
  globalVariablesLabel,
}: {
  variables: WorkflowVariableCatalogEntry[];
  globalVariablesLabel: string;
}) {
  return (
    <>
      {groupWorkflowVariables(variables, globalVariablesLabel).map((group) => (
        <SelectGroup key={group.label}>
          <SelectLabel className="px-2 pt-1.5 pb-0.5 text-[11px] font-medium text-muted-foreground">
            {group.label}
          </SelectLabel>
          {group.variables.map((variable) => {
            const selector = variable.selector.join(".");
            return (
              <SelectItem
                key={selector}
                value={selector}
                aria-label={selector}
                className="[&_[data-workflow-variable-part=variable-mark]]:text-blue-600! [&_[data-workflow-variable-part=variable-name]]:text-foreground! [&_[data-workflow-variable-part=variable-type]]:text-muted-foreground! dark:[&_[data-workflow-variable-part=variable-mark]]:text-blue-400!"
              >
                <WorkflowVariableRowContent variable={variable} />
              </SelectItem>
            );
          })}
        </SelectGroup>
      ))}
    </>
  );
}
