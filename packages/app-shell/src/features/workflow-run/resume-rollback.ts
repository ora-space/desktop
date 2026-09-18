import type { TFunction } from "i18next";

/** Backend reason strings that disable `node_files` / `checkpoint`. */
export const RESUME_ROLLBACK_UNAVAILABLE_REASONS = [
  "no_file_changes",
  "composite_region",
  "no_checkpoint",
  "siblings_ran_after_checkpoint",
  "not_resumable",
] as const;

export type ResumeRollbackUnavailableReason =
  (typeof RESUME_ROLLBACK_UNAVAILABLE_REASONS)[number];

/** Translation keys for each rollback-unavailability reason. */
export const ROLLBACK_UNAVAILABLE_KEYS = {
  no_file_changes: "workflowRun.resume.rollbackUnavailable.no_file_changes",
  composite_region: "workflowRun.resume.rollbackUnavailable.composite_region",
  no_checkpoint: "workflowRun.resume.reason.no_checkpoint",
  siblings_ran_after_checkpoint:
    "workflowRun.resume.reason.siblings_ran_after_checkpoint",
  not_resumable: "workflowRun.resume.reason.not_resumable",
} as const;

/** Maps a rollback unavailability reason onto the matching translated explanation. */
export function rollbackUnavailableReasonText(
  reason: string | null | undefined,
  t: TFunction,
): string | null {
  if (reason === null || reason === undefined) {
    return null;
  }
  if (reason in ROLLBACK_UNAVAILABLE_KEYS) {
    return t(
      ROLLBACK_UNAVAILABLE_KEYS[reason as ResumeRollbackUnavailableReason],
    );
  }
  return null;
}
