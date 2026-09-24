import { useTranslation } from "react-i18next";
import type { WorkflowNodeRetryWait } from "@ora/workflow-runtime";
import { retryWaitText, useRetryCountdown } from "./retry-countdown";

interface RunRetryWaitLabelProps {
  wait: WorkflowNodeRetryWait;
  /** `full` names the attempt; `compact` is only the countdown, for chips next to a title. */
  variant?: "full" | "compact";
  className?: string;
}

/**
 * Live "waiting to retry" label. Keyed by `dueAt`, so a later wait of the same node (the next
 * retry after another failure) starts from a fresh clock instead of the previous countdown.
 */
export function RunRetryWaitLabel({
  wait,
  variant = "full",
  className,
}: RunRetryWaitLabelProps) {
  return (
    <RetryWaitCountdown
      key={wait.dueAt}
      wait={wait}
      variant={variant}
      className={className}
    />
  );
}

function RetryWaitCountdown({
  wait,
  variant,
  className,
}: Required<Omit<RunRetryWaitLabelProps, "className">> & {
  className?: string;
}) {
  const { t } = useTranslation();
  const seconds = useRetryCountdown(wait.dueAt);
  return (
    <span
      className={className}
      data-retry-wait={variant}
      // The text changes every second; announcing each tick would drown out a screen reader.
      aria-live="off"
    >
      {retryWaitText(t, wait, seconds, variant)}
    </span>
  );
}
