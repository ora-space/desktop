import type {
  WorkflowFileReference,
  WorkflowVariableValueType,
} from "./node-data";

export const WORKFLOW_VARIABLE_VALUE_TYPES: WorkflowVariableValueType[] = [
  "string",
  "number",
  "integer",
  "boolean",
  "secret",
  "file",
  "object",
  "any",
  "array",
  "array[string]",
  "array[number]",
  "array[object]",
  "array[boolean]",
  "array[file]",
  "array[any]",
];

export type WorkflowVariableValueIssue =
  "invalid_boolean" | "invalid_file" | "invalid_json" | "invalid_type";

export type WorkflowVariableValueResult =
  | { valid: true; value: unknown }
  | { valid: false; issue: WorkflowVariableValueIssue };

/** Parses editor text and returns a normalized value only when it matches the declared type. */
export function parseWorkflowVariableValueText(
  text: string,
  valueType: WorkflowVariableValueType,
): WorkflowVariableValueResult {
  if (text === "") return { valid: true, value: undefined };
  if (valueType === "string" || valueType === "secret") {
    return { valid: true, value: text };
  }
  if (valueType === "file")
    return normalizeWorkflowVariableValue(text, valueType);
  if (valueType === "boolean") {
    if (text === "true") return { valid: true, value: true };
    if (text === "false") return { valid: true, value: false };
    return { valid: false, issue: "invalid_boolean" };
  }
  if (valueType === "number" || valueType === "integer") {
    const pattern =
      valueType === "integer"
        ? /^-?(?:0|[1-9]\d*)$/
        : /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/;
    const value = Number(text);
    return pattern.test(text) &&
      Number.isFinite(value) &&
      (valueType !== "integer" || Number.isInteger(value))
      ? { valid: true, value }
      : { valid: false, issue: "invalid_type" };
  }
  try {
    return normalizeWorkflowVariableValue(
      JSON.parse(text) as unknown,
      valueType,
    );
  } catch {
    return valueType === "any"
      ? { valid: true, value: text }
      : { valid: false, issue: "invalid_json" };
  }
}

/** Normalizes file references and verifies every scalar or array item against its declaration. */
export function normalizeWorkflowVariableValue(
  value: unknown,
  valueType: WorkflowVariableValueType,
): WorkflowVariableValueResult {
  if (valueType === "file") {
    const file = normalizeFileReference(value);
    return file === null
      ? { valid: false, issue: "invalid_file" }
      : { valid: true, value: file };
  }
  if (valueType === "array[file]") {
    if (!Array.isArray(value)) return { valid: false, issue: "invalid_type" };
    const files: WorkflowFileReference[] = [];
    for (const item of value) {
      const file = normalizeFileReference(item);
      if (file === null) return { valid: false, issue: "invalid_file" };
      files.push(file);
    }
    return { valid: true, value: files };
  }
  return workflowVariableValueMatchesType(value, valueType)
    ? { valid: true, value }
    : { valid: false, issue: "invalid_type" };
}

/** Checks a normalized value against one workflow variable declaration. */
export function workflowVariableValueMatchesType(
  value: unknown,
  valueType: WorkflowVariableValueType,
): boolean {
  if (valueType === "any") return true;
  if (valueType === "string" || valueType === "secret") {
    return typeof value === "string";
  }
  if (valueType === "file") return normalizeFileReference(value) !== null;
  if (valueType === "number")
    return typeof value === "number" && Number.isFinite(value);
  if (valueType === "integer")
    return typeof value === "number" && Number.isInteger(value);
  if (valueType === "boolean") return typeof value === "boolean";
  if (valueType === "object") return isRecord(value);
  if (valueType === "array" || valueType === "array[any]")
    return Array.isArray(value);
  if (!Array.isArray(value)) return false;
  if (valueType === "array[string]") {
    return value.every((item) => typeof item === "string");
  }
  if (valueType === "array[number]") {
    return value.every(
      (item) => typeof item === "number" && Number.isFinite(item),
    );
  }
  if (valueType === "array[boolean]") {
    return value.every((item) => typeof item === "boolean");
  }
  if (valueType === "array[object]") return value.every(isRecord);
  return value.every((item) => normalizeFileReference(item) !== null);
}

/** Formats normalized values back into the compact text accepted by the workflow editors. */
export function formatWorkflowVariableValue(
  value: unknown,
  valueType: WorkflowVariableValueType,
): string {
  if (value === undefined) return "";
  if (
    valueType === "file" &&
    isRecord(value) &&
    typeof value.path === "string"
  ) {
    return value.path;
  }
  if (valueType === "array[file]" && Array.isArray(value)) {
    return JSON.stringify(
      value.map((item) =>
        isRecord(item) && typeof item.path === "string" ? item.path : item,
      ),
    );
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** Returns one editor-friendly value that is guaranteed to satisfy the declared type. */
export function workflowVariableValueExample(
  valueType: WorkflowVariableValueType,
): string {
  switch (valueType) {
    case "string":
      return "text";
    case "number":
      return "1.5";
    case "integer":
      return "1";
    case "boolean":
      return "true";
    case "secret":
      return "token-value";
    case "file":
      return "docs/input.pdf";
    case "object":
    case "any":
      return '{"key":"value"}';
    case "array":
    case "array[any]":
      return '[1,"text",true]';
    case "array[string]":
      return '["one","two"]';
    case "array[number]":
      return "[1,2.5]";
    case "array[object]":
      return '[{"key":"value"}]';
    case "array[boolean]":
      return "[true,false]";
    case "array[file]":
      return '["docs/one.pdf","images/two.png"]';
  }
}

/** Converts a path string or canonical object into a safe Workspace-relative reference. */
function normalizeFileReference(value: unknown): WorkflowFileReference | null {
  const rawPath =
    typeof value === "string"
      ? value
      : isRecord(value) &&
          value.kind === "workspace_file" &&
          typeof value.path === "string"
        ? value.path
        : null;
  if (rawPath === null || rawPath === "" || /^[A-Za-z]:/.test(rawPath))
    return null;
  if (rawPath.startsWith("/") || rawPath.startsWith("\\")) return null;
  const segments: string[] = [];
  for (const segment of rawPath.split(/[\\/]/)) {
    if (segment === "" || segment === ".") continue;
    if (
      segment === ".." ||
      /^[A-Za-z]:/.test(segment) ||
      isWindowsReservedName(segment)
    ) {
      return null;
    }
    segments.push(segment);
  }
  if (segments.length === 0) return null;
  return { kind: "workspace_file", path: segments.join("/") };
}

/** Mirrors the backend's portable-path rejection for Win32 device aliases. */
function isWindowsReservedName(segment: string): boolean {
  const stem = (segment.split(".")[0] ?? "")
    .replace(/[ .]+$/, "")
    .toUpperCase();
  return (
    ["CON", "PRN", "AUX", "NUL"].includes(stem) || /^(COM|LPT)[1-9]$/.test(stem)
  );
}

/** Narrows JSON-like values to non-array objects. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
