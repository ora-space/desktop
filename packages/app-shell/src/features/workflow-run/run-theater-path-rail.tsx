import { useTranslation } from "react-i18next";
import { useMemo, type RefObject } from "react";
import { cn } from "@ora/ui";
import { RunRetryWaitLabel } from "./run-retry-wait-label";
import { retryWaitAttemptText } from "./retry-countdown";
import { RunStatusMark } from "./run-status-mark";
import { runStatusTone } from "./run-status-style";
import { type GraphWorkflowRun, type HitlRequest } from "@ora/workflow-runtime";
import {
  projectRunPathStructure,
  type RunPathRegionStage,
} from "./run-path-structure";
import { RunTheaterRegionNavigator } from "./run-theater-region-navigator";
import "./theater-motion.css";

interface RunTheaterPathRailProps {
  run: GraphWorkflowRun;
  primaryId: string | null;
  activeIds: readonly string[];
  openHitls: readonly HitlRequest[];
  artifactCountByNode: Readonly<Record<string, number>>;
  selectedRound?: number | null;
  onRoundChange?: (round: number) => void;
  /** When true, no path chip is marked current (result act has the stage). */
  showResultAct: boolean;
  pathRailRef: RefObject<HTMLDivElement | null>;
  onFocusNode: (nodeId: string) => void;
  onExpandHitl: (requestId: string) => void;
  /** Terminal path review → back to the result act. */
  onShowResultAct?: () => void;
}

/**
 * Theater header: outer progress path plus the selected iteration's nested phases.
 * Waiting chips expand HITL; all other chips change the focused act.
 */
export function RunTheaterPathRail({
  run,
  primaryId,
  activeIds,
  openHitls,
  artifactCountByNode,
  selectedRound = null,
  onRoundChange,
  showResultAct,
  pathRailRef,
  onFocusNode,
  onExpandHitl,
  onShowResultAct,
}: RunTheaterPathRailProps) {
  const { t } = useTranslation();
  const nodeById = useMemo(
    () => new Map(run.definitionSnapshot.nodes.map((node) => [node.id, node])),
    [run.definitionSnapshot.nodes],
  );
  const pathStages = useMemo(
    () =>
      projectRunPathStructure(run.definitionSnapshot).filter(
        (stage) => run.nodeStates[stage.nodeId]?.status !== "inactive",
      ),
    [run.definitionSnapshot, run.nodeStates],
  );
  const activeRegion = useMemo(
    () =>
      pathStages.find(
        (stage): stage is RunPathRegionStage =>
          stage.type === "region" &&
          (stage.nodeId === primaryId ||
            stage.phases.some((phase) =>
              phase.nodeIds.includes(primaryId ?? ""),
            )),
      ) ?? null,
    [pathStages, primaryId],
  );
  const progress = useMemo(() => {
    const total = Math.max(pathStages.length, 1);
    const done = pathStages.filter((stage) => {
      const status = run.nodeStates[stage.nodeId]?.status;
      return (
        status === "succeeded" || status === "failed" || status === "cancelled"
      );
    }).length;
    return { done, total, percent: Math.round((done / total) * 100) };
  }, [pathStages, run.nodeStates]);
  const activeIdSet = useMemo(() => new Set(activeIds), [activeIds]);
  const hitlByNodeId = useMemo(
    () => new Map(openHitls.map((request) => [request.nodeId, request])),
    [openHitls],
  );
  const terminal = onShowResultAct !== undefined;

  return (
    <div
      ref={pathRailRef}
      className="shrink-0 border-b border-border/80 bg-muted/20 px-4 py-3"
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-2.5">
        <div className="flex items-center justify-between gap-3">
          <p className="text-[11px] font-medium uppercase tracking-[0.05em] text-muted-foreground">
            {t("workflowRun.theater.path")}
          </p>
          <p className="text-[11px] tabular-nums text-muted-foreground">
            {t("workflowRun.progressValue", {
              done: progress.done,
              total: progress.total,
            })}
          </p>
        </div>
        <div
          className="h-1.5 overflow-hidden rounded-full bg-muted"
          role="progressbar"
          aria-valuenow={progress.percent}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label={t("workflowRun.field.progress")}
        >
          <div
            className={cn(
              "relative h-full overflow-hidden rounded-full bg-foreground/75 transition-[width] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none",
              run.status === "running" && "bg-sky-600/80",
              run.status === "awaiting_input" && "bg-amber-600/80",
              run.status === "succeeded" && "bg-emerald-600/75",
              run.status === "failed" && "bg-rose-600/75",
              run.status === "cancelled" && "bg-zinc-500/60",
            )}
            style={{ width: `${progress.percent}%` }}
          >
            {(run.status === "running" || run.status === "awaiting_input") && (
              <span
                className="theater-progress-sheen absolute inset-0"
                aria-hidden
              />
            )}
          </div>
        </div>
        <div className="overflow-x-auto" data-slot="theater-path-rail">
          <ol
            className="flex w-max gap-2 pb-0.5"
            aria-label={t("workflowRun.theater.topLevelPath")}
            data-slot="theater-top-level-path"
          >
            {pathStages.map((stage) => {
              const node = nodeById.get(stage.nodeId);
              if (node === undefined) return null;
              const state = run.nodeStates[stage.nodeId] ?? {
                status: "idle" as const,
              };
              const tone = runStatusTone(state.status);
              const containsPrimary =
                stage.type === "region" &&
                stage.phases.some((phase) =>
                  phase.nodeIds.includes(primaryId ?? ""),
                );
              const selected =
                !showResultAct &&
                (stage.nodeId === primaryId || containsPrimary);
              const waiting = state.status === "awaiting_input";
              const retryWait =
                state.status === "retry_waiting" ? state.retryWait : undefined;
              const active = activeIdSet.has(stage.nodeId);
              const nodeArtifactCount = artifactCountByNode[stage.nodeId] ?? 0;
              const roundCount =
                stage.type === "region" ? countRegionRounds(run, stage) : 0;
              return (
                <li key={stage.nodeId}>
                  <button
                    type="button"
                    data-path-node={stage.nodeId}
                    data-contains-current={containsPrimary ? "" : undefined}
                    data-waiting={waiting ? "" : undefined}
                    data-retry-waiting={
                      state.status === "retry_waiting" ? "" : undefined
                    }
                    onClick={() => {
                      const gate = hitlByNodeId.get(stage.nodeId);
                      if (gate !== undefined) {
                        onExpandHitl(gate.id);
                        return;
                      }
                      onFocusNode(stage.nodeId);
                    }}
                    className={cn(
                      "inline-flex max-w-[11rem] cursor-pointer items-center gap-2 rounded-full border px-2.5 py-1.5 text-left transition-[transform,colors,box-shadow] duration-200",
                      selected && waiting
                        ? "theater-chip-pop border-amber-500/55 bg-amber-500/15 text-amber-950 shadow-sm dark:text-amber-50"
                        : selected && state.status === "retry_waiting"
                          ? "theater-chip-pop border-orange-500/55 bg-background shadow-sm"
                          : selected
                            ? "theater-chip-pop border-foreground/35 bg-background shadow-sm"
                            : waiting
                              ? "border-amber-500/40 bg-amber-500/10 text-amber-950 dark:text-amber-100"
                              : state.status === "retry_waiting"
                                ? "border-orange-500/40 bg-orange-500/10 text-orange-950 dark:text-orange-100"
                                : active
                                  ? "border-sky-500/40 bg-sky-500/10"
                                  : "border-transparent bg-background/60 hover:border-border hover:bg-background",
                    )}
                    aria-current={selected ? "step" : undefined}
                    aria-label={`${node.data.title}: ${
                      retryWait !== undefined
                        ? retryWaitAttemptText(t, retryWait)
                        : t(tone.labelKey)
                    }`}
                  >
                    <RunStatusMark status={state.status} quiet />
                    <span className="truncate font-sans text-[11px] font-medium">
                      {node.data.title}
                    </span>
                    {retryWait !== undefined && (
                      <RunRetryWaitLabel
                        wait={retryWait}
                        variant="compact"
                        className="shrink-0 text-[9px] tabular-nums text-orange-800 dark:text-orange-200"
                      />
                    )}
                    {stage.type === "region" && (
                      <span className="shrink-0 text-[9px] tabular-nums text-muted-foreground">
                        {roundCount > 0
                          ? t("workflowRun.theater.iterationChipSummary", {
                              members: stage.memberCount,
                              rounds: roundCount,
                            })
                          : t("workflowRun.theater.iterationMembers", {
                              members: stage.memberCount,
                            })}
                      </span>
                    )}
                    {nodeArtifactCount > 0 && (
                      <span
                        className="shrink-0 tabular-nums text-[9px] text-muted-foreground"
                        aria-label={t("workflowRun.artifacts.countBadge", {
                          count: nodeArtifactCount,
                        })}
                      >
                        {nodeArtifactCount}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
            {terminal && (
              <li>
                <button
                  type="button"
                  data-path-result=""
                  onClick={onShowResultAct}
                  className={cn(
                    "inline-flex max-w-[11rem] cursor-pointer items-center gap-2 rounded-full border px-2.5 py-1.5 text-left transition-[transform,colors,box-shadow] duration-200",
                    showResultAct
                      ? cn(
                          "theater-chip-pop bg-background shadow-sm",
                          run.status === "succeeded" && "border-emerald-500/45",
                          run.status === "failed" && "border-rose-500/45",
                          run.status === "cancelled" && "border-zinc-400/45",
                        )
                      : cn(
                          "bg-background/60 hover:bg-background",
                          run.status === "succeeded" &&
                            "border-emerald-500/25 hover:border-emerald-500/40",
                          run.status === "failed" &&
                            "border-rose-500/25 hover:border-rose-500/40",
                          run.status === "cancelled" &&
                            "border-zinc-400/25 hover:border-zinc-400/40",
                        ),
                  )}
                  aria-current={showResultAct ? "step" : undefined}
                  aria-label={`${t("workflowRun.result.pathChip")}: ${t(runStatusTone(run.status).labelKey)}`}
                >
                  <RunStatusMark status={run.status} quiet />
                  <span className="truncate font-sans text-[11px] font-medium">
                    {t("workflowRun.result.pathChip")}
                  </span>
                </button>
              </li>
            )}
          </ol>
        </div>
        {activeRegion !== null && (
          <RunTheaterRegionNavigator
            run={run}
            region={activeRegion}
            primaryId={primaryId}
            selectedRound={selectedRound}
            artifactCountByNode={artifactCountByNode}
            onRoundChange={onRoundChange}
            onFocusNode={onFocusNode}
          />
        )}
      </div>
    </div>
  );
}

/** Counts distinct persisted rounds represented by an iteration's members. */
function countRegionRounds(
  run: GraphWorkflowRun,
  region: RunPathRegionStage,
): number {
  const rounds = new Set<number>();
  for (const nodeId of region.phases.flatMap((phase) => phase.nodeIds)) {
    for (const state of run.roundStates?.[nodeId] ?? []) {
      if (state.iteration !== undefined) rounds.add(state.iteration);
    }
  }
  return rounds.size;
}
