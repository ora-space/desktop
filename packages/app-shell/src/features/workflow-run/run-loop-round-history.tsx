import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import type { GraphWorkflowRound } from "@ora/workflow-runtime";
import { retryWaitAttemptText } from "./retry-countdown";
import { RunRetryWaitLabel } from "./run-retry-wait-label";
import { RunStatusBadge } from "./run-status-mark";
import { loopRoundLabel } from "./loop-round-state";

/** Lets a Loop inspector select one persisted round without merging repeated child states. */
export function RunLoopRoundHistory({
  rounds,
  nodeTitles,
  selectedRoundId,
  onSelectedRoundChange,
}: {
  rounds: GraphWorkflowRound[];
  nodeTitles: Record<string, string>;
  selectedRoundId?: string | null;
  onSelectedRoundChange?: (roundId: string) => void;
}) {
  const { t } = useTranslation();
  const safeRounds = rounds ?? [];
  const hasMultipleAttempts =
    new Set(safeRounds.map((round) => round.parentLoopNodeRunId)).size > 1;
  const [preferredRoundId, setPreferredRoundId] = useState<string | null>(null);
  const selectedRound =
    selectedRoundId === undefined
      ? (safeRounds.find((round) => round.id === preferredRoundId) ??
        [...safeRounds].sort((a, b) => a.roundIndex - b.roundIndex)[0])
      : safeRounds.find((round) => round.id === selectedRoundId);

  const members = Object.entries(selectedRound?.nodeStates ?? {});
  if (safeRounds.length === 0) {
    return (
      <p className="text-[11px] leading-5 text-muted-foreground">
        {t("workflowRun.loopRounds.empty")}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-1.5" role="tablist">
        {safeRounds.map((round) => {
          const selected = round.id === selectedRound?.id;
          return (
            <button
              key={round.id}
              type="button"
              role="tab"
              aria-selected={selected}
              className={cn(
                "rounded-md border px-2 py-1 text-[10px] font-medium tabular-nums transition-colors",
                selected
                  ? "border-foreground/35 bg-muted text-foreground"
                  : "border-border text-muted-foreground hover:bg-muted/60",
              )}
              onClick={() => {
                if (selectedRoundId === undefined)
                  setPreferredRoundId(round.id);
                onSelectedRoundChange?.(round.id);
              }}
            >
              {loopRoundLabel(
                t("workflowRun.loopRounds.round", {
                  round: round.roundIndex,
                }),
                round,
                hasMultipleAttempts,
              )}
            </button>
          );
        })}
      </div>
      <div className="space-y-1.5">
        {members.map(([nodeId, state]) => (
          <div
            key={nodeId}
            className="flex min-w-0 items-center gap-2 rounded-lg border border-border bg-muted/20 px-2.5 py-2"
          >
            <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
              {nodeTitles[nodeId] ?? nodeId}
            </span>
            {state.nodeRunId && (
              <code
                className="truncate text-[10px] text-muted-foreground"
                title={state.nodeRunId}
              >
                {state.nodeRunId}
              </code>
            )}
            {state.sessionId !== undefined && (
              <span
                className="max-w-20 truncate font-mono text-[9px] text-muted-foreground"
                title={state.sessionId}
              >
                {state.sessionId}
              </span>
            )}
            {state.status === "retry_waiting" &&
              state.retryWait !== undefined && (
                <>
                  {/* Only the countdown is visible; screen readers also get the attempt count. */}
                  <span className="sr-only">
                    {retryWaitAttemptText(t, state.retryWait)}
                  </span>
                  <RunRetryWaitLabel
                    wait={state.retryWait}
                    variant="compact"
                    className="shrink-0 text-[9px] tabular-nums text-orange-700 dark:text-orange-300"
                  />
                </>
              )}
            <RunStatusBadge status={state.status} quiet className="shrink-0" />
          </div>
        ))}
      </div>
    </div>
  );
}
