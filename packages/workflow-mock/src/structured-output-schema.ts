import { WORKFLOW_VARIABLE_VALUE_TYPES } from "./variable-value";

export type WorkflowStructuredOutputSchemaValidation =
  { valid: true } | { valid: false; path: string; message: string };

/** Validates the JSON Schema subset understood by the workflow execution engine. */
export function validateWorkflowStructuredOutputSchema(
  schema: unknown,
): WorkflowStructuredOutputSchemaValidation {
  const result = validateSchemaNode(schema, "$");
  if (!result.valid) return result;
  return isRecord(schema) && schema.type === "object"
    ? { valid: true }
    : { valid: false, path: "$", message: "root type must be object" };
}

/** Recursively verifies one schema fragment and its object or array children. */
function validateSchemaNode(
  schema: unknown,
  path: string,
): WorkflowStructuredOutputSchemaValidation {
  if (!isRecord(schema) || typeof schema.type !== "string") {
    return { valid: false, path, message: "type must be a string" };
  }
  if (
    schema.type !== "null" &&
    !WORKFLOW_VARIABLE_VALUE_TYPES.includes(
      schema.type as (typeof WORKFLOW_VARIABLE_VALUE_TYPES)[number],
    )
  ) {
    return { valid: false, path, message: `unsupported type ${schema.type}` };
  }
  if (schema.properties !== undefined) {
    if (schema.type !== "object" || !isRecord(schema.properties)) {
      return {
        valid: false,
        path,
        message: "properties is only valid for object fields",
      };
    }
    for (const [name, property] of Object.entries(schema.properties)) {
      const result = validateSchemaNode(property, `${path}.properties.${name}`);
      if (!result.valid) return result;
    }
  }
  if (schema.required !== undefined) {
    if (
      schema.type !== "object" ||
      !Array.isArray(schema.required) ||
      !schema.required.every((entry) => typeof entry === "string")
    ) {
      return {
        valid: false,
        path,
        message: "required must be a string array on an object field",
      };
    }
    const properties = isRecord(schema.properties) ? schema.properties : {};
    const undeclared = schema.required.find((name) => !(name in properties));
    if (undeclared !== undefined) {
      return {
        valid: false,
        path,
        message: `required property ${undeclared} is not declared`,
      };
    }
  }
  if (
    schema.additionalProperties !== undefined &&
    (schema.type !== "object" ||
      typeof schema.additionalProperties !== "boolean")
  ) {
    return {
      valid: false,
      path,
      message: "additionalProperties must be a boolean on an object field",
    };
  }
  if (schema.items !== undefined) {
    if (schema.type !== "array") {
      return { valid: false, path, message: "items is only valid for arrays" };
    }
    return validateSchemaNode(schema.items, `${path}.items`);
  }
  return { valid: true };
}

/** Narrows JSON-like values to non-array objects. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
