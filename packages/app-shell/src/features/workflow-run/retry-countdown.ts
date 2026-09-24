import { useEffect, useState } from "react";
import type { useTranslation } from "react-i18next";
import type { WorkflowNodeRetryWait } from "@ora/workflow-runtime";

type Translate = ReturnType<typeof useTranslation>["t"];

/** Whole seconds left until `dueAt`, rounded up so the label reaches 0 at the deadline itself. */
export function retryCountdownSeconds(dueAt: number, now: number): number {
  return Math.max(0, Math.ceil((dueAt - now) / 1_000));
}

/**
 * Live countdown to a waiting retry's `due_at`, recomputed from the clock each second.
 *
 * Each timer is armed for the moment the rounded-up value drops, so the label never lags the
 * deadline by up to a second. Nothing is armed at zero: the backend starts the attempt and the
 * next run refresh replaces the waiting row. Callers key the owner by `dueAt` so a new wait
 * starts from a fresh clock reading.
 */
export function useRetryCountdown(dueAt: number): number {
  const [now, setNow] = useState(Date.now);
  const seconds = retryCountdownSeconds(dueAt, now);
  useEffect(() => {
    if (seconds <= 0) {
      return;
    }
    const delay = Math.max(0, dueAt - now - (seconds - 1) * 1_000);
    const timer = window.setTimeout(() => setNow(Date.now()), delay);
    return () => window.clearTimeout(timer);
  }, [dueAt, now, seconds]);
  return seconds;
}

/** Static part of the waiting label, used where a live countdown cannot be announced. */
export function retryWaitAttemptText(
  t: Translate,
  wait: WorkflowNodeRetryWait,
): string {
  return t("workflowRun.retryWait.attempt", {
    attempt: wait.attempt,
    max: wait.maxAttempt,
  });
}

/**
 * Waiting label with its countdown. `full` is the node-card form; `compact` is the short
 * countdown that path chips append after the node title.
 */
export function retryWaitText(
  t: Translate,
  wait: WorkflowNodeRetryWait,
  seconds: number,
  variant: "full" | "compact",
): string {
  if (variant === "compact") {
    return seconds > 0
      ? t("workflowRun.retryWait.countdown", { seconds })
      : t("workflowRun.retryWait.starting");
  }
  return seconds > 0
    ? t("workflowRun.retryWait.label", {
        attempt: wait.attempt,
        max: wait.maxAttempt,
        seconds,
      })
    : t("workflowRun.retryWait.labelStarting", {
        attempt: wait.attempt,
        max: wait.maxAttempt,
      });
}
