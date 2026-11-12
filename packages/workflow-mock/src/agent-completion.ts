import type {
  WorkflowAgentCompletion,
  WorkflowAgentCompletionFile,
} from "./node-data";

/** Maximum number of declarations one `agentConfig.completion` may carry. */
export const WORKFLOW_AGENT_COMPLETION_MAX_FILES = 32;

/** Maximum UTF-8 byte length of one declared completion path or glob. */
export const WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES = 512;

/** Maximum UTF-8 byte and UTF-16 code-unit length of one path segment. */
const MAX_SEGMENT_LENGTH = 255;

/** Maximum nested directories allowed below the workspace root. */
const MAX_DEPTH = 32;

const UNSUPPORTED_GLOB_CHARACTERS = /[[\]{}]/u;
const CONTROL_CHARACTERS = /[\p{Cc}\u007f]/u;
const WINDOWS_DRIVE_PREFIX = /^[A-Za-z]:/u;

/** One machine-readable reason a declared completion path is invalid. */
export type WorkflowAgentCompletionPathIssue =
  | "empty"
  | "tooLong"
  | "controlCharacter"
  | "backslash"
  | "unsupportedGlobCharacter"
  | "negation"
  | "absolute"
  | "emptySegment"
  | "parentTraversal"
  | "drivePrefix"
  | "reservedName"
  | "invalidGlobstar"
  | "segmentTooLong"
  | "tooDeep";

/** Narrows one unknown value to a plain record without accepting arrays. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Matches the backend's Windows reserved-device rule for one path segment. */
function isReservedDeviceName(segment: string): boolean {
  const stem = (segment.split(".")[0] ?? segment).replace(/[ .]+$/u, "");
  return /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])$/iu.test(stem);
}

/**
 * Validates one declared completion path with the same minimal glob rules as the backend.
 *
 * Returns a stable code rather than user-facing copy so the inspector can localize the problem
 * without leaking validator internals into product text.
 */
export function workflowAgentCompletionPathIssue(
  path: string,
): WorkflowAgentCompletionPathIssue | null {
  if (path === "") {
    return "empty";
  }
  if (
    new TextEncoder().encode(path).length >
    WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES
  ) {
    return "tooLong";
  }
  if (CONTROL_CHARACTERS.test(path)) {
    return "controlCharacter";
  }
  if (path.includes("\\")) {
    return "backslash";
  }
  if (UNSUPPORTED_GLOB_CHARACTERS.test(path)) {
    return "unsupportedGlobCharacter";
  }
  if (path.startsWith("!")) {
    return "negation";
  }
  if (
    path.startsWith("/") ||
    path.startsWith("//") ||
    WINDOWS_DRIVE_PREFIX.test(path)
  ) {
    return "absolute";
  }
  const segments = path.split("/");
  for (const segment of segments) {
    if (segment === "" || segment === ".") {
      return "emptySegment";
    }
    if (segment === "..") {
      return "parentTraversal";
    }
    if (WINDOWS_DRIVE_PREFIX.test(segment)) {
      return "drivePrefix";
    }
    if (isReservedDeviceName(segment)) {
      return "reservedName";
    }
    if (segment.includes("**") && segment !== "**") {
      return "invalidGlobstar";
    }
    if (
      new TextEncoder().encode(segment).length > MAX_SEGMENT_LENGTH ||
      segment.length > MAX_SEGMENT_LENGTH
    ) {
      return "segmentTooLong";
    }
  }
  if (segments.length - 1 > MAX_DEPTH) {
    return "tooDeep";
  }
  return null;
}

/** Returns a technical English detail for import failures; UI copy is translated separately. */
function pathIssueDetail(issue: WorkflowAgentCompletionPathIssue): string {
  switch (issue) {
    case "empty":
      return "path must not be empty";
    case "tooLong":
      return `path must not exceed ${WORKFLOW_AGENT_COMPLETION_MAX_PATH_BYTES} UTF-8 bytes`;
    case "controlCharacter":
      return "path must not contain control characters";
    case "backslash":
      return "path must use '/' and must not contain a backslash";
    case "unsupportedGlobCharacter":
      return "path may only use the glob characters *, **, and ?";
    case "negation":
      return "path must not use glob negation";
    case "absolute":
      return "path must be relative to the workspace";
    case "emptySegment":
      return "path must not contain empty or '.' segments";
    case "parentTraversal":
      return "path must not contain '..' segments";
    case "drivePrefix":
      return "path must not contain a Windows drive prefix";
    case "reservedName":
      return "path must not use a Windows reserved device name";
    case "invalidGlobstar":
      return "the ** wildcard must be a complete path segment";
    case "segmentTooLong":
      return `path segments must not exceed ${MAX_SEGMENT_LENGTH} bytes or UTF-16 units`;
    case "tooDeep":
      return `path must not nest deeper than ${MAX_DEPTH} directories`;
  }
}

/**
 * Validates an optional `agentConfig.completion` value.
 *
 * Unknown fields are reported instead of dropped so a typo cannot silently disable a gate. An
 * absent or null declaration is valid and produces no issues; `isWorkflowAgentCompletion` remains
 * the stricter type guard for a value that is actually declared.
 */
export function workflowAgentCompletionIssues(value: unknown): string[] {
  if (value === undefined || value === null) {
    return [];
  }
  if (!isRecord(value)) {
    return ["completion must be an object"];
  }
  const extraKeys = Object.keys(value).filter((key) => key !== "files");
  if (extraKeys.length > 0) {
    return [
      `completion does not support ${extraKeys.map((key) => `'${key}'`).join(", ")}; only files is defined`,
    ];
  }
  if (!Array.isArray(value.files)) {
    return ["completion.files must be an array"];
  }
  if (
    value.files.length < 1 ||
    value.files.length > WORKFLOW_AGENT_COMPLETION_MAX_FILES
  ) {
    return [
      `completion.files must contain between 1 and ${WORKFLOW_AGENT_COMPLETION_MAX_FILES} entries`,
    ];
  }
  const issues: string[] = [];
  value.files.forEach((file, index) => {
    if (!isRecord(file)) {
      issues.push(`completion.files[${index}] must be an object`);
      return;
    }
    const extra = Object.keys(file).filter(
      (key) => key !== "path" && key !== "mustChange",
    );
    if (extra.length > 0) {
      issues.push(
        `completion.files[${index}] does not support ${extra.map((key) => `'${key}'`).join(", ")}`,
      );
      return;
    }
    if (typeof file.path !== "string") {
      issues.push(`completion.files[${index}].path must be a string`);
      return;
    }
    const pathIssue = workflowAgentCompletionPathIssue(file.path);
    if (pathIssue !== null) {
      issues.push(
        `completion.files[${index}].path: ${pathIssueDetail(pathIssue)}`,
      );
    }
    if (file.mustChange !== undefined && typeof file.mustChange !== "boolean") {
      issues.push(`completion.files[${index}].mustChange must be a boolean`);
    }
  });
  return issues;
}

/** Returns whether the value is a declared, valid completion object. */
export function isWorkflowAgentCompletion(
  value: unknown,
): value is WorkflowAgentCompletion {
  return (
    value !== undefined &&
    value !== null &&
    isRecord(value) &&
    workflowAgentCompletionIssues(value).length === 0
  );
}

/** Preserves an editable declaration without silently dropping fields or entries. */
export function normalizeWorkflowAgentCompletion(
  value: unknown,
): WorkflowAgentCompletion | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  return value as unknown as WorkflowAgentCompletion;
}

/** Returns one declaration's mustChange flag with its documented default. */
export function workflowAgentCompletionFileMustChange(
  file: WorkflowAgentCompletionFile,
): boolean {
  return file.mustChange === true;
}
