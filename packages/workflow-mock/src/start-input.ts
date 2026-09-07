import type {
  WorkflowInputFieldType,
  WorkflowInputVariable,
  WorkflowVariableValueType,
} from "./node-data";

/** Start field types shown by the editor, in the same order as the configuration menu. */
export const WORKFLOW_INPUT_FIELD_TYPES = [
  "text-input",
  "paragraph",
  "select",
  "number",
  "checkbox",
  "file",
  "file-list",
  "json",
] as const satisfies readonly WorkflowInputFieldType[];

/** Returns the variable-pool type produced by one Start form control. */
export function workflowInputFieldValueType(
  fieldType: WorkflowInputFieldType,
): WorkflowVariableValueType {
  switch (fieldType) {
    case "text-input":
    case "paragraph":
    case "select":
      return "string";
    case "number":
      return "number";
    case "checkbox":
      return "boolean";
    case "file":
      return "file";
    case "file-list":
      return "array[file]";
    case "json":
      return "object";
  }
}

/** Resolves legacy Start declarations that predate explicit form control metadata. */
export function resolveWorkflowInputFieldType(
  variable: Pick<WorkflowInputVariable, "fieldType" | "valueType">,
): WorkflowInputFieldType {
  if (variable.fieldType !== undefined) return variable.fieldType;
  switch (variable.valueType) {
    case "number":
    case "integer":
      return "number";
    case "boolean":
      return "checkbox";
    case "file":
      return "file";
    case "array[file]":
      return "file-list";
    case "object":
    case "any":
    case "array":
    case "array[string]":
    case "array[number]":
    case "array[object]":
    case "array[boolean]":
    case "array[any]":
      return "json";
    case "string":
    case "secret":
      return "text-input";
  }
}
