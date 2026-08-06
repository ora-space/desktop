import { useTranslation } from "react-i18next";
import { Button, cn } from "@ora/ui";
import {
  IconBan,
  IconCheck,
  IconMap,
  IconX,
} from "@tabler/icons-react";
import { RunStatusBadge } from "./run-status-mark";
import { runStatusTone } from "./run-status-style";
import type { GraphWorkflowRun, GraphWorkflowRunStatus } from "@ora/workflow-runtime";

interface RunResultActProps {
  run: GraphWorkflowRun;
  artifactCount: number;
  onShowOverview: () => void;
  onOpenArtifacts?: () => void;
}

/**
 * End-of-run Theater surface when focus is not pinned to a history act.
 * Path chips still open single-node review; Run again stays in the header.
 */
export function RunResultAct({
  run,
  artifactCount,
  onShowOverview,
  onOpenArtifacts,
}: RunResultActProps) {
  const { t } = useTranslation();
  const tone = runStatusTone(run.status);
  const titleKey = `workflowRun.result.title.${run.status}` as const;
  const bodyKey = `workflowRun.result.body.${run.status}` as const;

  return (
    <div
      className={cn(
        "mx-auto w-full max-w-xl",
        "animate-in fade-in zoom-in-95 slide-in-from-bottom-2",
        "duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] fill-mode-both",
        "motion-reduce:animate-none",
      )}
    >
      <section
        className={cn(
          "rounded-2xl border bg-card p-6",
          "shadow-[0_1px_2px_rgba(0,0,0,0.04),0_8px_24px_rgba(0,0,0,0.04)]",
          "dark:shadow-[0_1px_2px_rgba(0,0,0,0.28),0_10px_28px_rgba(0,0,0,0.16)]",
          tone.ring,
          "ring-1",
        )}
        aria-live="polite"
      >
        <div className="flex items-start gap-3">
          <ResultHeroMark status={run.status} />
          <div className="min-w-0 flex-1 space-y-2">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="text-base font-semibold tracking-tight text-foreground">
                {t(titleKey)}
              </h2>
              <RunStatusBadge status={run.status} quiet />
            </div>
            <p className="text-sm leading-6 text-muted-foreground">
              {t(bodyKey)}
            </p>
            <p className="truncate text-xs text-muted-foreground/80">
              {run.name}
            </p>
          </div>
        </div>

        <dl className="mt-5 grid grid-cols-2 gap-2 sm:grid-cols-3">
          <div className="rounded-lg border border-border/70 bg-muted/20 px-3 py-2.5">
            <dt className="text-[10px] text-muted-foreground">
              {t("workflowRun.field.duration")}
            </dt>
            <dd className="mt-0.5 text-xs tabular-nums">
              {run.totals.durationMs !== undefined
                ? t("workflowRun.totalsDuration", { ms: run.totals.durationMs })
                : "—"}
            </dd>
          </div>
          <div className="rounded-lg border border-border/70 bg-muted/20 px-3 py-2.5">
            <dt className="text-[10px] text-muted-foreground">
              {t("workflowRun.field.tokens")}
            </dt>
            <dd className="mt-0.5 text-xs tabular-nums">
              {run.totals.tokenUsage?.totalTokens !== undefined
                ? t("workflowRun.totalsTokens", {
                  count: run.totals.tokenUsage.totalTokens,
                })
                : "—"}
            </dd>
          </div>
          <div className="col-span-2 rounded-lg border border-border/70 bg-muted/20 px-3 py-2.5 sm:col-span-1">
            <dt className="text-[10px] text-muted-foreground">
              {t("workflowRun.artifacts.title")}
            </dt>
            <dd className="mt-0.5 text-xs tabular-nums">
              {t("workflowRun.artifacts.countBadge", { count: artifactCount })}
            </dd>
          </div>
        </dl>

        <div className="mt-5 flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="cursor-pointer gap-1.5"
            onClick={onShowOverview}
          >
            <IconMap className="size-3.5" />
            {t("workflowRun.result.showOverview")}
          </Button>
          {artifactCount > 0 && onOpenArtifacts !== undefined && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="cursor-pointer"
              onClick={onOpenArtifacts}
            >
              {t("workflowRun.result.openArtifacts")}
            </Button>
          )}
        </div>

        <p className="mt-4 text-[11px] leading-5 text-muted-foreground">
          {t("workflowRun.result.pathHint")}
        </p>
      </section>
    </div>
  );
}

/** Single hero circle —avoids nesting RunStatusMark's own disc. */
function ResultHeroMark({ status }: { status: GraphWorkflowRunStatus }) {
  const glyphClass = "size-5";
  return (
    <span
      className={cn(
        "mt-0.5 flex size-10 shrink-0 items-center justify-center rounded-full text-white",
        status === "succeeded" && "bg-emerald-500",
        status === "failed" && "bg-rose-500",
        status === "cancelled" && "bg-zinc-400 dark:bg-zinc-500",
        (status === "pending"
          || status === "running"
          || status === "awaiting_input")
          && "bg-muted text-muted-foreground",
      )}
      aria-hidden
    >
      {status === "succeeded" && <IconCheck className={glyphClass} stroke={2.5} />}
      {status === "failed" && <IconX className={glyphClass} stroke={2.5} />}
      {status === "cancelled" && <IconBan className={glyphClass} stroke={2.5} />}
    </span>
  );
}
