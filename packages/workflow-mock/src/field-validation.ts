import type {
  WorkflowConditionComparison,
  WorkflowVariableValueType,
} from "./node-data";
import { isWorkflowConditionComparisonComplete } from "./node-data";
import type { WorkflowChoice } from "./capabilities";
import type { WorkflowVariableCatalogEntry } from "./variable-catalog";

/** An array-valued catalog entry can be authored as an iteration source, including a schema path. */
export function isIteratorSource(entry: WorkflowVariableCatalogEntry) {
  return entry.valueType.startsWith("array");
}
/** Collection targets are root outputs of direct iteration members supplied by the caller. */
export function isCollectOutput(entry: WorkflowVariableCatalogEntry) {
  return (
    entry.selector.length === 2 &&
    ["output", "structured_output"].includes(entry.selector[1]!)
  );
}
/** Conservative declaration-level filter. The runtime validates each actual feedback value. */
export function supportsFeedback(
  target: WorkflowVariableValueType,
  source: WorkflowVariableValueType,
) {
  return (
    target === source ||
    (["string", "secret"].includes(target) &&
      ["string", "secret"].includes(source)) ||
    (target === "object" && source === "file") ||
    (target === "array[object]" && source === "array[file]") ||
    target === "any" ||
    source === "any" ||
    (target === "number" && source === "integer") ||
    ((target === "array" || target === "array[any]") &&
      source.startsWith("array"))
  );
}

/** Groups operators by the types of operands they can evaluate. */
function operandFamily(operator: string) {
  switch (operator) {
    case "greater_than":
    case "greater_than_or_equal":
    case "less_than":
    case "less_than_or_equal":
      return "number";
    case "starts_with":
    case "ends_with":
      return "text";
    case "contains":
    case "not_contains":
      return "contains";
    default:
      return "any";
  }
}

/** Offers comparisons appropriate for the selected value, preserving unknown-type editing. */
export function operatorSupportsType(
  operator: string,
  valueType: WorkflowVariableValueType | undefined,
): boolean {
  if (valueType === undefined || valueType === "any") return true;
  switch (operandFamily(operator)) {
    case "number":
      return valueType === "number" || valueType === "integer";
    case "contains":
      return (
        ["string", "secret"].includes(valueType) ||
        valueType.startsWith("array")
      );
    case "text":
      return ["string", "secret"].includes(valueType);
    default:
      return true;
  }
}
/** Resolves the complete selector against the catalog for this execution scope. */
export function findVariable(
  catalog: WorkflowVariableCatalogEntry[],
  selector: string[],
) {
  return catalog.find(
    (entry) =>
      entry.selector.length === selector.length &&
      entry.selector.every((part, i) => part === selector[i]),
  );
}

/** Validates all serialized variable tokens in a prompt or comparison template. */
export function validReferences(
  value: unknown,
  catalog: WorkflowVariableCatalogEntry[],
) {
  if (typeof value !== "string") return true;
  return value
    .split("{{#")
    .slice(1)
    .every((tail) => {
      const end = tail.indexOf("#}}");
      return (
        end > 0 &&
        findVariable(catalog, tail.slice(0, end).split(".")) !== undefined
      );
    });
}

/** A sole token keeps its declared type; surrounding text renders a string. */
function validOperand(
  rule: WorkflowConditionComparison,
  source: WorkflowVariableCatalogEntry,
  variables: WorkflowVariableCatalogEntry[],
) {
  if (!validReferences(rule.value, variables)) return false;
  if (rule.value === undefined) return true;
  const token =
    typeof rule.value === "string" &&
    /^\{\{#([^{}#]+)#\}\}$/.exec(rule.value.trim());
  const type = token
    ? findVariable(variables, token[1]!.split("."))?.valueType
    : typeof rule.value;
  if (type === "any") return true;
  const family = operandFamily(rule.operator);
  if (family === "number") return type === "integer" || type === "number";
  if (
    family === "text" ||
    (family === "contains" && ["string", "secret"].includes(source.valueType))
  ) {
    return type === "string" || type === "secret";
  }
  return true;
}

/** Stored runtime operators can predate the narrower menu of newly offered operators. */
export function invalidComparisonField(
  rule: WorkflowConditionComparison,
  variables: WorkflowVariableCatalogEntry[],
  operators: WorkflowChoice[],
): "variable" | "operator" | "value" | undefined {
  const variable = findVariable(variables, rule.variableSelector);
  if (!variable) return "variable";
  const supported =
    operators.some((op) => op.value === rule.operator) ||
    ["is", "is_not", "exists", "not_exists"].includes(rule.operator);
  if (!supported || !operatorSupportsType(rule.operator, variable.valueType))
    return "operator";
  if (
    !isWorkflowConditionComparisonComplete(rule) ||
    !validOperand(rule, variable, variables)
  )
    return "value";
}
