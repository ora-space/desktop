import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import type { GraphWorkflowRound } from "@ora/workflow-runtime";
import { retryWaitAttemptText } from "./retry-countdown";
import { RunRetryWaitLabel } from "./run-retry-wait-label";
import { RunStatusBadge } from "./run-status-mark";

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
  const [preferredRoundId, setPreferredRoundId] = useState<string | null>(null);
  const effectiveRoundId = selectedRoundId ?? preferredRoundId;
  const selectedRound =
    rounds.find((round) => round.id === effectiveRoundId) ?? rounds.at(-1);

  if (selectedRound === undefined) {
    return (
      <p className="text-[11px] leading-5 text-muted-foreground">
        {t("workflowRun.loopRounds.empty")}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-1.5" role="tablist">
        {rounds.map((round) => {
          const selected = round.id === selectedRound.id;
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
                setPreferredRoundId(round.id);
                onSelectedRoundChange?.(round.id);
              }}
            >
              {t("workflowRun.loopRounds.round", {
                round: round.roundIndex + 1,
              })}
            </button>
          );
        })}
      </div>
      <div className="space-y-1.5">
        {Object.entries(selectedRound.nodeStates).map(([nodeId, state]) => (
          <div
            key={nodeId}
            className="flex min-w-0 items-center gap-2 rounded-lg border border-border bg-muted/20 px-2.5 py-2"
          >
            <span className="min-w-0 flex-1 truncate text-[11px] font-medium">
              {nodeTitles[nodeId] ?? nodeId}
            </span>
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
