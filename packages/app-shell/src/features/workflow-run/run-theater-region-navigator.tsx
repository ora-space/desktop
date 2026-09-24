import { useState } from "react";
import { IconChevronLeft, IconChevronRight } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { cn, Popover, PopoverContent, PopoverTrigger } from "@ora/ui";
import type {
  GraphWorkflowNodeState,
  GraphWorkflowRun,
} from "@ora/workflow-runtime";
import { RunRetryWaitLabel } from "./run-retry-wait-label";
import { retryWaitAttemptText } from "./retry-countdown";
import { RunStatusMark } from "./run-status-mark";
import { runStatusTone } from "./run-status-style";
import type { RunPathRegionStage } from "./run-path-structure";
import { formatElapsedDuration } from "../../lib/format";

interface RunTheaterRegionNavigatorProps {
  run: GraphWorkflowRun;
  region: RunPathRegionStage;
  primaryId: string | null;
  selectedRound: number | null;
  artifactCountByNode: Readonly<Record<string, number>>;
  onRoundChange?: (round: number) => void;
  onFocusNode: (nodeId: string) => void;
}

/** Persistent iteration hierarchy shown below the outer Theater path. */
export function RunTheaterRegionNavigator({
  run,
  region,
  primaryId,
  selectedRound,
  artifactCountByNode,
  onRoundChange,
  onFocusNode,
}: RunTheaterRegionNavigatorProps) {
  const { t } = useTranslation();
  const [roundMenuOpen, setRoundMenuOpen] = useState(false);
  const nodeById = new Map(
    run.definitionSnapshot.nodes.map((node) => [node.id, node]),
  );
  const regionNode = nodeById.get(region.nodeId);
  const memberIds = region.phases.flatMap((phase) => phase.nodeIds);
  const rounds = availableRounds(run, memberIds);
  const effectiveRound =
    selectedRound !== null && rounds.includes(selectedRound)
      ? selectedRound
      : (rounds[rounds.length - 1] ?? null);
  const roundPosition =
    effectiveRound === null ? -1 : rounds.indexOf(effectiveRound);
  const regionTitle = regionNode?.data.title ?? region.nodeId;
  const regionLabel =
    effectiveRound === null
      ? regionTitle
      : t("workflowRun.theater.iterationNavigatorLabel", {
          name: regionTitle,
          round: effectiveRound + 1,
          total: rounds.length,
        });
  const completedMembers =
    effectiveRound === null
      ? 0
      : memberIds.filter((nodeId) => {
          const status = stateForRound(run, nodeId, effectiveRound)?.status;
          return (
            status === "succeeded" ||
            status === "failed" ||
            status === "cancelled"
          );
        }).length;

  return (
    <section
      className="rounded-xl border border-violet-500/20 bg-violet-500/[0.035] p-2.5"
      role="region"
      aria-label={regionLabel}
      data-iteration-region={region.nodeId}
    >
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-[11px] font-semibold text-foreground">
            {regionTitle}
          </p>
          <div className="flex items-center gap-1.5 text-[9px] text-muted-foreground">
            <span>
              {rounds.length > 0
                ? t("workflowRun.theater.iterationSummary", {
                    members: region.memberCount,
                    rounds: rounds.length,
                  })
                : t("workflowRun.theater.iterationMembers", {
                    members: region.memberCount,
                  })}
            </span>
            {effectiveRound !== null && (
              <span>
                {t("workflowRun.theater.iterationRoundProgress", {
                  done: completedMembers,
                  total: memberIds.length,
                })}
              </span>
            )}
          </div>
        </div>
        {effectiveRound !== null && rounds.length > 1 && (
          <div
            className="flex shrink-0 items-center gap-1"
            role="group"
            aria-label={t("workflowRun.inspector.rounds")}
          >
            <button
              type="button"
              className="flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground disabled:opacity-35"
              aria-label={t("workflowRun.theater.previousRound")}
              disabled={roundPosition <= 0}
              onClick={() => {
                const previous = rounds[roundPosition - 1];
                if (previous !== undefined) onRoundChange?.(previous);
              }}
            >
              <IconChevronLeft className="size-3.5" />
            </button>
            <Popover open={roundMenuOpen} onOpenChange={setRoundMenuOpen}>
              <PopoverTrigger
                render={
                  <button
                    type="button"
                    className="min-w-16 rounded-md px-1.5 py-1 text-center text-[10px] font-medium tabular-nums text-violet-700 hover:bg-background dark:text-violet-300"
                    aria-label={t("workflowRun.theater.selectRound")}
                  />
                }
              >
                {t("workflowRun.theater.roundPosition", {
                  round: effectiveRound + 1,
                  total: rounds.length,
                })}
              </PopoverTrigger>
              <PopoverContent align="end" className="w-36 p-1.5">
                <div
                  className="flex max-h-56 flex-col gap-0.5 overflow-y-auto"
                  role="listbox"
                  aria-label={t("workflowRun.inspector.rounds")}
                >
                  {rounds.map((round) => (
                    <button
                      key={round}
                      type="button"
                      role="option"
                      aria-selected={round === effectiveRound}
                      className={cn(
                        "rounded-md px-2 py-1.5 text-left text-[10px] tabular-nums hover:bg-muted",
                        round === effectiveRound &&
                          "bg-violet-500/10 text-violet-700 dark:text-violet-300",
                      )}
                      onClick={() => {
                        onRoundChange?.(round);
                        setRoundMenuOpen(false);
                      }}
                    >
                      {t("workflowRun.theater.roundOption", {
                        round: round + 1,
                      })}
                    </button>
                  ))}
                </div>
              </PopoverContent>
            </Popover>
            <button
              type="button"
              className="flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground disabled:opacity-35"
              aria-label={t("workflowRun.theater.nextRound")}
              disabled={roundPosition >= rounds.length - 1}
              onClick={() => {
                const next = rounds[roundPosition + 1];
                if (next !== undefined) onRoundChange?.(next);
              }}
            >
              <IconChevronRight className="size-3.5" />
            </button>
          </div>
        )}
      </div>

      <div className="overflow-x-auto pb-0.5">
        <ol className="flex w-max items-stretch gap-2" aria-label={regionLabel}>
          {region.phases.map((phase, phaseIndex) => (
            <li key={phase.id} className="flex items-center gap-2">
              {phaseIndex > 0 && (
                <span className="text-xs text-muted-foreground/55" aria-hidden>
                  →
                </span>
              )}
              <div
                className={cn(
                  "relative flex gap-1.5 rounded-lg border px-1.5 pb-1.5 pt-5",
                  phase.kind === "parallel"
                    ? "border-sky-500/25 bg-sky-500/[0.035]"
                    : phase.kind === "conditional"
                      ? "border-amber-500/25 bg-amber-500/[0.035]"
                      : "border-transparent p-0",
                )}
                role="group"
                aria-label={phaseLabel(t, phase.kind, phase.nodeIds.length)}
                data-region-phase-kind={phase.kind}
              >
                {phase.kind !== "single" && (
                  <span className="absolute left-2 top-1 text-[9px] font-medium text-muted-foreground">
                    {phaseLabel(t, phase.kind, phase.nodeIds.length)}
                  </span>
                )}
                {phase.nodeIds.map((nodeId) => {
                  const node = nodeById.get(nodeId);
                  if (node === undefined) return null;
                  const state = stateForRound(run, nodeId, effectiveRound);
                  const notRun = state === null;
                  const tone = runStatusTone(
                    state?.status ?? ("inactive" as const),
                  );
                  const selected = primaryId === nodeId;
                  const artifactCount = artifactCountByNode[nodeId] ?? 0;
                  const duration = stateDuration(state);
                  const retryWait =
                    state?.status === "retry_waiting"
                      ? state.retryWait
                      : undefined;
                  const stateLabel = notRun
                    ? t("workflowRun.theater.notRunThisRound")
                    : retryWait !== undefined
                      ? retryWaitAttemptText(t, retryWait)
                      : t(tone.labelKey);
                  return (
                    <button
                      key={nodeId}
                      type="button"
                      data-path-node={nodeId}
                      aria-current={selected ? "step" : undefined}
                      aria-label={`${node.data.title}: ${stateLabel}`}
                      onClick={() => onFocusNode(nodeId)}
                      className={cn(
                        "flex min-w-32 max-w-44 items-center gap-2 rounded-md border bg-background/80 px-2 py-1.5 text-left transition-colors",
                        selected
                          ? "border-violet-500/45 shadow-sm"
                          : retryWait !== undefined
                            ? "border-orange-500/40 hover:border-orange-500/55"
                            : "border-border/65 hover:border-violet-500/30",
                        notRun && "opacity-60",
                      )}
                    >
                      <RunStatusMark
                        status={state?.status ?? "inactive"}
                        quiet
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[10px] font-medium">
                          {node.data.title}
                        </span>
                        {retryWait !== undefined ? (
                          <RunRetryWaitLabel
                            wait={retryWait}
                            variant="compact"
                            className="block text-[9px] tabular-nums text-orange-700 dark:text-orange-300"
                          />
                        ) : (
                          duration !== null && (
                            <span className="block text-[9px] tabular-nums text-muted-foreground">
                              {duration}
                            </span>
                          )
                        )}
                      </span>
                      {artifactCount > 0 && (
                        <span className="text-[9px] tabular-nums text-muted-foreground">
                          {artifactCount}
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}

/** Formats one completed member state's elapsed duration when both endpoints exist. */
function stateDuration(state: GraphWorkflowNodeState | null): string | null {
  if (state?.startedAt === undefined || state.finishedAt === undefined) {
    return null;
  }
  const startedAt = Date.parse(state.startedAt);
  const finishedAt = Date.parse(state.finishedAt);
  if (!Number.isFinite(startedAt) || !Number.isFinite(finishedAt)) {
    return null;
  }
  return formatElapsedDuration(finishedAt - startedAt);
}

/** Returns all persisted rounds represented by any member in this region. */
function availableRounds(run: GraphWorkflowRun, memberIds: string[]): number[] {
  const rounds = new Set<number>();
  for (const nodeId of memberIds) {
    for (const state of run.roundStates?.[nodeId] ?? []) {
      if (state.iteration !== undefined) rounds.add(state.iteration);
    }
  }
  return [...rounds].sort((left, right) => left - right);
}

/** Resolves only the requested round so an older/newer result cannot leak into the view. */
function stateForRound(
  run: GraphWorkflowRun,
  nodeId: string,
  round: number | null,
): GraphWorkflowNodeState | null {
  if (round === null) {
    return run.nodeStates[nodeId] ?? null;
  }
  return (
    run.roundStates?.[nodeId]?.find((state) => state.iteration === round) ??
    null
  );
}

/** Provides the visible and accessible label for one structural phase. */
function phaseLabel(
  t: ReturnType<typeof useTranslation>["t"],
  kind: RunPathRegionStage["phases"][number]["kind"],
  count: number,
): string {
  if (kind === "parallel") {
    return t("workflowRun.theater.parallelGroup", { count });
  }
  if (kind === "conditional") {
    return t("workflowRun.theater.conditionalGroup", { count });
  }
  return t("workflowRun.theater.sequentialGroup");
}
