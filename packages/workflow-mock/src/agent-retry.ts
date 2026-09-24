import type { WorkflowAgentRetryPolicy } from "./node-data";

/**
 * Policy the engine applies when an Agent node has no `agentConfig.retry`. Kept here, next to the
 * bounds, so the editor, the run view, and validation never disagree about the defaults.
 */
export const DEFAULT_WORKFLOW_AGENT_RETRY: Readonly<WorkflowAgentRetryPolicy> =
  Object.freeze({
    enabled: true,
    maxRetries: 2,
    initialDelaySeconds: 10,
  });

/** Numeric retry settings; each must be an integer within `WORKFLOW_AGENT_RETRY_BOUNDS`. */
export type WorkflowAgentRetryNumericField =
  "maxRetries" | "initialDelaySeconds";

/** Inclusive integer bounds the engine accepts for each numeric retry setting. */
export const WORKFLOW_AGENT_RETRY_BOUNDS: Readonly<
  Record<WorkflowAgentRetryNumericField, Readonly<{ min: number; max: number }>>
> = Object.freeze({
  maxRetries: Object.freeze({ min: 0, max: 5 }),
  initialDelaySeconds: Object.freeze({ min: 0, max: 300 }),
});

/** Cap on any single wait: the wait before retry n is `initialDelaySeconds × 2^(n-1)`. */
export const WORKFLOW_AGENT_RETRY_MAX_DELAY_SECONDS = 600;

/** Why one numeric retry setting would be rejected by the engine. */
export interface WorkflowAgentRetryNumericIssue {
  field: WorkflowAgentRetryNumericField;
  reason: "missing" | "notInteger" | "outOfRange";
}

/**
 * One reason a stored `retry` value would be rejected by the engine. `field` names the offending
 * setting so editors can place the message next to it; `null` means the value is not an object.
 */
export type WorkflowAgentRetryIssue =
  | { field: null; reason: "notObject" }
  | { field: "enabled"; reason: "missing" | "notBoolean" }
  | WorkflowAgentRetryNumericIssue;

/** Numeric fields in display order, shared by validation and the editor. */
export const WORKFLOW_AGENT_RETRY_NUMERIC_FIELDS: readonly WorkflowAgentRetryNumericField[] =
  ["maxRetries", "initialDelaySeconds"];

/**
 * Returns the policy the engine applies to this Agent configuration: the stored value, or the
 * default when the field is absent (`null` counts as absent, matching the backend decoder).
 * Always returns a fresh object so callers may spread or edit it.
 */
export function resolveWorkflowAgentRetryPolicy(config: {
  retry?: WorkflowAgentRetryPolicy | null;
}): WorkflowAgentRetryPolicy {
  const retry = config.retry ?? DEFAULT_WORKFLOW_AGENT_RETRY;
  return {
    enabled: retry.enabled,
    maxRetries: retry.maxRetries,
    initialDelaySeconds: retry.initialDelaySeconds,
  };
}

/** Interactive Agent nodes wait for a human instead of rerunning, so retry never applies to them. */
export function workflowAgentRetryApplies(config: {
  interactive?: boolean;
}): boolean {
  return config.interactive !== true;
}

/**
 * Lists every reason the engine would reject a stored `retry` value; an empty list means it is
 * accepted. Absent (`undefined` or `null`) is valid and means the default policy.
 */
export function validateWorkflowAgentRetry(
  value: unknown,
): WorkflowAgentRetryIssue[] {
  if (value === undefined || value === null) {
    return [];
  }
  if (typeof value !== "object" || Array.isArray(value)) {
    return [{ field: null, reason: "notObject" }];
  }
  const record = value as Record<string, unknown>;
  const issues: WorkflowAgentRetryIssue[] = [];
  if (record.enabled === undefined || record.enabled === null) {
    issues.push({ field: "enabled", reason: "missing" });
  } else if (typeof record.enabled !== "boolean") {
    issues.push({ field: "enabled", reason: "notBoolean" });
  }
  for (const field of WORKFLOW_AGENT_RETRY_NUMERIC_FIELDS) {
    const candidate = record[field];
    const bounds = WORKFLOW_AGENT_RETRY_BOUNDS[field];
    if (candidate === undefined || candidate === null) {
      issues.push({ field, reason: "missing" });
    } else if (typeof candidate !== "number" || !Number.isInteger(candidate)) {
      issues.push({ field, reason: "notInteger" });
    } else if (candidate < bounds.min || candidate > bounds.max) {
      issues.push({ field, reason: "outOfRange" });
    }
  }
  return issues;
}
