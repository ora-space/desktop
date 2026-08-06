import { useTranslation } from "react-i18next";
import { useMemo, type RefObject } from "react";
import { cn } from "@ora/ui";
import { RunStatusMark } from "./run-status-mark";
import { runStatusTone } from "./run-status-style";
import type { GraphWorkflowRun, HitlRequest } from "@ora/workflow-runtime";
import "./theater-motion.css";

interface RunTheaterPathRailProps {
  run: GraphWorkflowRun;
  primaryId: string | null;
  activeIds: readonly string[];
  openHitls: readonly HitlRequest[];
  artifactCountByNode: Readonly<Record<string, number>>;
  /** When true, no path chip is marked current (result act has the stage). */
  showResultAct: boolean;
  progress: { done: number; total: number; percent: number };
  pathRailRef: RefObject<HTMLDivElement | null>;
  onFocusNode: (nodeId: string) => void;
  onExpandHitl: (requestId: string) => void;
  /** Terminal path review → back to the result act. */
  onShowResultAct?: () => void;
}

/**
 * Theater header: progress track + horizontal path chips.
 * Waiting chips expand HITL; others only change focus.
 */
export function RunTheaterPathRail({
  run,
  primaryId,
  activeIds,
  openHitls,
  artifactCountByNode,
  showResultAct,
  progress,
  pathRailRef,
  onFocusNode,
  onExpandHitl,
  onShowResultAct,
}: RunTheaterPathRailProps) {
  const { t } = useTranslation();
  const activeIdSet = useMemo(() => new Set(activeIds), [activeIds]);
  const hitlByNodeId = useMemo(
    () => new Map(openHitls.map((request) => [request.nodeId, request])),
    [openHitls],
  );
  const terminal = onShowResultAct !== undefined;

  return (
    <div className="shrink-0 border-b border-border/80 bg-muted/20 px-4 py-3">
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
              run.status === "failed"
                && "bg-rose-600/75",
              run.status === "cancelled" && "bg-zinc-500/60",
            )}
            style={{ width: `${progress.percent}%` }}
          >
            {(run.status === "running" || run.status === "awaiting_input") && (
              <span className="theater-progress-sheen absolute inset-0" aria-hidden />
            )}
          </div>
        </div>
        <div className="overflow-x-auto" ref={pathRailRef} data-slot="theater-path-rail">
          <ol className="flex w-max gap-2 pb-0.5">
            {run.definitionSnapshot.nodes.map((node) => {
              const state = run.nodeStates[node.id] ?? { status: "idle" as const };
              const tone = runStatusTone(state.status);
              const selected = !showResultAct && node.id === primaryId;
              const waiting = state.status === "awaiting_input";
              const active = activeIdSet.has(node.id);
              const nodeArtifactCount = artifactCountByNode[node.id] ?? 0;
              return (
                <li key={node.id}>
                  <button
                    type="button"
                    data-path-node={node.id}
                    data-waiting={waiting ? "" : undefined}
                    onClick={() => {
                      const gate = hitlByNodeId.get(node.id);
                      if (gate !== undefined) {
                        onExpandHitl(gate.id);
                        return;
                      }
                      onFocusNode(node.id);
                    }}
                    className={cn(
                      "inline-flex max-w-[11rem] cursor-pointer items-center gap-2 rounded-full border px-2.5 py-1.5 text-left transition-[transform,colors,box-shadow] duration-200",
                      selected && waiting
                        ? "theater-chip-pop border-amber-500/55 bg-amber-500/15 text-amber-950 shadow-sm dark:text-amber-50"
                        : selected
                        ? "theater-chip-pop border-foreground/35 bg-background shadow-sm"
                        : waiting
                        ? "border-amber-500/40 bg-amber-500/10 text-amber-950 dark:text-amber-100"
                        : active
                        ? "border-sky-500/40 bg-sky-500/10"
                        : "border-transparent bg-background/60 hover:border-border hover:bg-background",
                    )}
                    aria-current={selected ? "step" : undefined}
                    aria-label={`${node.data.title}: ${t(tone.labelKey)}`}
                  >
                    <RunStatusMark status={state.status} quiet />
                    <span className="truncate text-[11px] font-medium">
                      {node.data.title}
                    </span>
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
                        run.status === "succeeded" && "border-emerald-500/25 hover:border-emerald-500/40",
                        run.status === "failed" && "border-rose-500/25 hover:border-rose-500/40",
                        run.status === "cancelled" && "border-zinc-400/25 hover:border-zinc-400/40",
                      ),
                  )}
                  aria-current={showResultAct ? "step" : undefined}
                  aria-label={`${t("workflowRun.result.pathChip")}: ${t(runStatusTone(run.status).labelKey)}`}
                >
                  <RunStatusMark status={run.status} quiet />
                  <span className="truncate text-[11px] font-medium">
                    {t("workflowRun.result.pathChip")}
                  </span>
                </button>
              </li>
            )}
          </ol>
        </div>
      </div>
    </div>
  );
}
