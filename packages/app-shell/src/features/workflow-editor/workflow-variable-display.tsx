import { IconVariable } from "@tabler/icons-react";
import type {
  WorkflowNodeKind,
  WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import { getNodeMetadata } from "../workflow-node-chrome";

/** A catalog entry carrying the optional producing-node kind the editor adds for display. */
export type EditorWorkflowVariable = WorkflowVariableCatalogEntry & {
  sourceNodeKind?: WorkflowNodeKind;
};

type DisplayWorkflowVariable = Pick<
  WorkflowVariableCatalogEntry,
  "variableName"
> & {
  sourceNodeKind?: WorkflowNodeKind;
};

/** Reads the producing-node kind an editor-enriched catalog entry may carry. */
function variableSourceKind(
  variable: DisplayWorkflowVariable,
): WorkflowNodeKind | undefined {
  return variable.sourceNodeKind;
}

/**
 * Renders a variable the way Dify labels it: node icon + node title on the left,
 * then the `{x}` variable name. Node identity is foreground-colored and the
 * variable name uses the same blue as the variable dropdown.
 */
export function WorkflowVariableDisplay({
  variable,
  nodeName,
}: {
  variable: DisplayWorkflowVariable;
  nodeName: string;
}) {
  const kind = variableSourceKind(variable);
  const Icon = kind !== undefined ? getNodeMetadata(kind).icon : IconVariable;
  return (
    <span className="inline-flex max-w-full min-w-0 items-center gap-1 align-middle text-xs">
      <Icon
        aria-hidden="true"
        data-workflow-variable-part="node-icon"
        className="size-3.5 shrink-0 text-foreground"
      />
      <span
        data-workflow-variable-part="node-name"
        className="truncate font-medium text-foreground"
      >
        {nodeName}
      </span>
      <span aria-hidden="true" className="shrink-0 text-muted-foreground">
        /
      </span>
      <span
        aria-hidden="true"
        data-workflow-variable-part="variable-mark"
        className="shrink-0 font-medium text-blue-600 dark:text-blue-400"
      >
        {"{x}"}
      </span>
      <span
        data-workflow-variable-part="variable-name"
        className="truncate font-medium text-blue-600 dark:text-blue-400"
      >
        {variable.variableName}
      </span>
    </span>
  );
}
