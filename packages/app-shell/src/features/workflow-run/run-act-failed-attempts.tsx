import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import { formatRunClock } from "../../lib/format";
import { NODE_FAILURE_KINDS } from "./node-failure-kinds";
import type {
  WorkflowNodeAttemptFailure,
  WorkflowNodeAttemptReplacement,
} from "@ora/workflow-runtime";

const KNOWN_NODE_FAILURE_KINDS = new Set<string>(NODE_FAILURE_KINDS);

const REPLACEMENT_LABEL_KEYS: Record<WorkflowNodeAttemptReplacement, string> = {
  automatic_retry: "workflowRun.failedAttempts.replacedByRetry",
  automatic_retry_scheduled:
    "workflowRun.failedAttempts.replacedByScheduledRetry",
  manual_resume: "workflowRun.failedAttempts.replacedByResume",
};

/**
 * Earlier failed attempts of the inspected node execution, oldest first. The current attempt keeps
 * its own error block above; this list only covers the attempts it replaced, so it renders
 * nothing when there are none.
 */
export function RunActFailedAttempts({
  attempts,
}: {
  attempts: readonly WorkflowNodeAttemptFailure[] | undefined;
}) {
  const { t } = useTranslation();
  if (attempts === undefined || attempts.length === 0) {
    return null;
  }
  return (
    <section className="space-y-2.5" data-slot="failed-attempts">
      <h4 className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
        {t("workflowRun.failedAttempts.title")}
      </h4>
      <ol className="space-y-2">
        {attempts.map((attempt) => (
          <FailedAttemptEntry key={attempt.nodeRunId} attempt={attempt} />
        ))}
      </ol>
    </section>
  );
}

function FailedAttemptEntry({
  attempt,
}: {
  attempt: WorkflowNodeAttemptFailure;
}) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const kindLabel = KNOWN_NODE_FAILURE_KINDS.has(attempt.kind)
    ? t(`workflowRun.errorKind.${attempt.kind}`)
    : attempt.kind;
  const roundLabel =
    attempt.iteration !== undefined
      ? t("workflowRun.theater.roundOption", { round: attempt.iteration + 1 })
      : attempt.loopRoundIndex !== undefined
        ? t("workflowRun.loopRounds.round", {
            round: attempt.loopRoundIndex + 1,
          })
        : null;
  // Attempts that never started a session carry only the time the failure was recorded.
  const timeLabel =
    attempt.startedAt !== undefined
      ? `${formatRunClock(attempt.startedAt, locale)} — ${
          attempt.finishedAt !== undefined
            ? formatRunClock(attempt.finishedAt, locale)
            : "—"
        }`
      : formatRunClock(new Date(attempt.recordedAt).toISOString(), locale);
  const rootCause = attempt.sourceChain.at(-1);

  return (
    <li
      className="space-y-1.5 rounded-lg border border-rose-500/20 bg-rose-500/[0.03] px-3 py-2 text-[11px] leading-5"
      data-attempt={attempt.attempt}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="font-medium text-foreground">
          {t("workflowRun.errorAttempt", { count: attempt.attempt })}
        </span>
        <span className="text-rose-700 dark:text-rose-300">{kindLabel}</span>
        {roundLabel !== null && (
          <span className="rounded bg-violet-500/12 px-1 py-0.5 text-[9px] font-medium tabular-nums text-violet-700 dark:text-violet-300">
            {roundLabel}
          </span>
        )}
        {attempt.replacedBy !== undefined && (
          <AttemptChip automatic={attempt.replacedBy === "automatic_retry"}>
            {t(REPLACEMENT_LABEL_KEYS[attempt.replacedBy])}
          </AttemptChip>
        )}
        {attempt.beforeRestart === true && (
          <AttemptChip automatic={false}>
            {t("workflowRun.failedAttempts.beforeRestart")}
          </AttemptChip>
        )}
        <span className="ml-auto text-[10px] tabular-nums text-muted-foreground/80">
          {timeLabel}
        </span>
      </div>
      <p className="whitespace-pre-wrap break-words text-foreground/90">
        {attempt.errorMessage}
      </p>
      {rootCause !== undefined && (
        <p className="whitespace-pre-wrap break-words text-muted-foreground">
          <span className="font-medium">
            {t("workflowRun.failedAttempts.rootCause")}
          </span>
          {": "}
          {rootCause}
        </p>
      )}
      {attempt.sourceChain.length > 1 && (
        <details>
          <summary className="cursor-pointer text-[10px] text-muted-foreground">
            {t("workflowRun.failedAttempts.chain", {
              levels: attempt.sourceChain.length,
            })}
          </summary>
          <ol className="mt-1 list-decimal space-y-0.5 pl-5 font-mono text-[10px] text-muted-foreground">
            {attempt.sourceChain.map((entry, index) => (
              // Chain entries can repeat verbatim, so the position is part of the key.
              <li key={`${index}:${entry}`} className="break-words">
                {entry}
              </li>
            ))}
          </ol>
        </details>
      )}
    </li>
  );
}

/** Automatic retries that ran share the waiting state's orange; the other chips stay neutral. */
function AttemptChip({
  automatic,
  children,
}: {
  automatic: boolean;
  children: string;
}) {
  return (
    <span
      className={cn(
        "rounded-full border px-1.5 py-px text-[9px] font-medium",
        automatic
          ? "border-orange-500/30 bg-orange-500/10 text-orange-800 dark:text-orange-300"
          : "border-border bg-muted/60 text-muted-foreground",
      )}
    >
      {children}
    </span>
  );
}
