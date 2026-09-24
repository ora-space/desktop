/** Wire values for `NodeFailureKind`; kept in one list so i18n tests can iterate them. */
export const NODE_FAILURE_KINDS = [
  "missing_agent_ref",
  "workflow_model_not_found",
  "missing_agent_config",
  "invalid_run_payload",
  "prompt_template",
  "structured_output",
  "missing_skill_materialization",
  "session_ended_without_stop_reason",
  "session_binding_rejected",
  "baseline_persist",
  "repository",
  "session",
  "agent_refusal",
  "unknown_stop_reason",
  "interrupted_by_restart",
  "multiple_outputs",
  "condition_evaluation",
] as const;

/**
 * Kinds a same-version rerun describes to the agent when the run injects previous failures;
 * mirrors `NodeFailureKind::inject_into_prompt`. A row of one of these kinds recorded with
 * `injects_previous_failure: false` belongs to a run created with that injection off.
 */
export const PROMPT_INJECTED_FAILURE_KINDS: ReadonlySet<string> = new Set([
  "structured_output",
  "agent_refusal",
  "unknown_stop_reason",
  "multiple_outputs",
]);

/**
 * Kinds whose `workflowRun.errorHint` text promises the retry will carry the failure; they also
 * have a `workflowRun.errorHintWithoutInjection` text for runs that inject nothing.
 */
export const HINT_PROMISES_INJECTION_KINDS: ReadonlySet<string> = new Set([
  "structured_output",
  "agent_refusal",
]);
