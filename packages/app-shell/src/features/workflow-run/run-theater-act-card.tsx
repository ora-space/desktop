import { useTranslation } from "react-i18next";
import { Badge, cn } from "@ora/ui";
import {
  createMockWorkflowNodeType,
  type WorkflowNodeData,
} from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import { WorkflowNodeCardShell } from "../workflow-node-chrome";
import { RunStatusBadge, isNodeWorking } from "./run-status-mark";
import { runStatusTone } from "./run-status-style";
import type { GraphWorkflowNodeState } from "./runtime/types";
import "./theater-motion.css";

interface RunTheaterActCardProps {
  nodeId: string;
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  /** Soft emphasis when this act is live (running / awaiting). */
  live: boolean;
  /** Glanceable outcome count; detail lives in the act inspector. */
  artifactCount?: number;
  /** Large primary stage vs secondary parallel card. */
  variant?: "stage" | "compact";
  /** Opens the act inspector (stage) or promotes a parallel act. */
  onSelect?: () => void;
  /** Stronger stage presence when this card is the focused parallel act. */
  emphasized?: boolean;
}

/**
 * Theater act card: instruction + metrics on the stage surface.
 * Clicking the primary card opens the companion inspector for full config
 * and outcomes.
 */
export function RunTheaterActCard({
  nodeId,
  data,
  state,
  live,
  artifactCount = 0,
  variant = "stage",
  onSelect,
  emphasized = true,
}: RunTheaterActCardProps) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const kindLabel = createMockWorkflowNodeType(data.kind, locale).label;
  const tone = runStatusTone(state.status);
  const detail = data.model ?? data.tool ?? data.condition;
  const compact = variant === "compact";
  const interactive = onSelect !== undefined;
  const timingRange = state.startedAt !== undefined || state.finishedAt !== undefined
    ? [
      state.startedAt !== undefined
        ? formatRunClock(state.startedAt, locale)
        : "—",
      state.finishedAt !== undefined
        ? formatRunClock(state.finishedAt, locale)
        : "—",
    ].join(" → ")
    : null;

  const metrics = (
    <div className="space-y-2.5">
      {timingRange !== null && (
        <p className="text-[10px] tabular-nums text-muted-foreground/65">
          {timingRange}
        </p>
      )}
      <dl className={cn("grid gap-3", compact ? "grid-cols-2" : "sm:grid-cols-3")}>
        {!compact && (
          <div className="rounded-lg border border-border/70 bg-background/80 px-3 py-2.5">
            <dt className="text-[10px] text-muted-foreground">
              {t("workflowRun.theater.nodeId")}
            </dt>
            <dd className="mt-0.5 truncate font-mono text-xs">{nodeId}</dd>
          </div>
        )}
        <div className="rounded-lg border border-border/70 bg-background/80 px-3 py-2.5">
          <dt className="text-[10px] text-muted-foreground">
            {t("workflowRun.field.duration")}
          </dt>
          <dd className="mt-0.5 text-xs tabular-nums">
            {state.durationMs !== undefined
              ? t("workflowRun.totalsDuration", { ms: state.durationMs })
              : "—"}
          </dd>
        </div>
        <div className="rounded-lg border border-border/70 bg-background/80 px-3 py-2.5">
          <dt className="text-[10px] text-muted-foreground">
            {t("workflowRun.field.tokens")}
          </dt>
          <dd className="mt-0.5 text-xs tabular-nums">
            {state.tokenUsage?.totalTokens !== undefined
              ? t("workflowRun.totalsTokens", {
                count: state.tokenUsage.totalTokens,
              })
              : "—"}
          </dd>
        </div>
      </dl>
    </div>
  );

  return (
    <WorkflowNodeCardShell
      kind={data.kind}
      title={data.title}
      description={data.description}
      kindLabel={kindLabel}
      density={compact ? "compact" : "stage"}
      className={cn(
        compact ? "w-full" : "mx-auto w-full max-w-xl",
        "transition-[border-color,box-shadow] duration-200 ease-out motion-reduce:transition-none",
        interactive
          && "cursor-pointer hover:border-foreground/25 hover:shadow-sm active:scale-[0.99]",
        emphasized && live && state.status === "running" && "theater-live-breathe",
        emphasized
          && live
          && state.status === "awaiting_input"
          && "theater-live-breathe-amber",
      )}
      ariaLabel={onSelect
        ? `${data.title}: ${t(tone.labelKey)}. ${t("workflowRun.theater.inspectorHint")}`
        : `${data.title}: ${t(tone.labelKey)}`}
      aria-live={compact ? undefined : "polite"}
      frameClassName={cn(
        tone.ring,
        "ring-1 transition-[box-shadow,ring-color] duration-300",
        live && state.status === "running" && "ring-sky-500/35",
        live && state.status === "awaiting_input" && "ring-amber-500/35",
      )}
      headerAccessory={(
        <div className="flex items-center gap-1.5">
          {artifactCount > 0 && (
            <Badge
              variant="secondary"
              className="tabular-nums text-[10px]"
            >
              {t("workflowRun.artifacts.countBadge", { count: artifactCount })}
            </Badge>
          )}
          <RunStatusBadge
            status={state.status}
            live={emphasized && isNodeWorking(state.status)}
          />
        </div>
      )}
      body={compact
        ? (
          <p className="mt-1 line-clamp-2 text-[11px] leading-4 text-muted-foreground">
            {data.description}
          </p>
        )
        : (
          <>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              {data.description}
            </p>
            <div className="mt-5 rounded-xl border border-border/80 bg-muted/30 px-4 py-3">
              <p className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
                {t("workflowRun.theater.instruction")}
              </p>
              <p className="mt-1.5 text-sm leading-6 text-foreground/90">
                {data.instruction}
              </p>
              {detail !== undefined && detail !== "" && (
                <p className="mt-2 font-mono text-[11px] text-muted-foreground">
                  {detail}
                </p>
              )}
            </div>
          </>
        )}
      footer={metrics}
      onClick={onSelect}
      onKeyDown={onSelect
        ? (event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onSelect();
          }
        }
        : undefined}
      role={onSelect ? "button" : undefined}
      tabIndex={onSelect ? 0 : undefined}
    />
  );
}
