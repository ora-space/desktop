import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Badge, Button, cn } from "@ora/ui";
import {
  IconArrowBackUp,
  IconMessageCircle,
  IconSparkles,
} from "@tabler/icons-react";
import {
  createMockWorkflowNodeType,
} from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import { WorkflowNodeCardShell } from "../workflow-node-chrome";
import { RunStatusBadge, isNodeWorking } from "./run-status-mark";
import { runStatusTone } from "./run-status-style";
import { RunNodeConversation } from "./run-node-conversation";
import type {
  GraphWorkflowNodeState,
  WorkflowNodeConversationItem,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
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
  /**
   * HITL surface for this act. Prefer the render form so the session dock can
   * live inside HITL chrome; a plain node falls back to a shared action strip.
   */
  interaction?: ReactNode | ((slots: { accessory: ReactNode | null }) => ReactNode);
  /** Filtered node session items; secondary activity is disclosed in-place. */
  conversation?: WorkflowNodeConversationItem[];
  /** Parallel peers opt in only for the focused card to keep carousel gestures stable. */
  conversationEnabled?: boolean;
}

type SessionDockTone = "stage" | "hitl";

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
  interaction,
  conversation = [],
  conversationEnabled = true,
}: RunTheaterActCardProps) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const kindLabel = createMockWorkflowNodeType(data.kind, locale).label;
  const tone = runStatusTone(state.status);
  const detail = data.model ?? data.tool ?? data.condition;
  const compact = variant === "compact";
  const [conversationOpen, setConversationOpen] = useState(false);
  const conversationMessageCount = useMemo(
    () => conversation.reduce(
      (count, item) => (item.kind === "message" ? count + 1 : count),
      0,
    ),
    [conversation],
  );
  const showDockSpark = conversationMessageCount === 0;
  // Keep the session dock available during HITL so readers can inspect prior
  // node messages before answering a permission or clarify gate.
  const canOpenConversation = !compact && conversationEnabled;
  const isConversationOpen = conversationOpen && canOpenConversation;
  const interactive = onSelect !== undefined && !isConversationOpen;
  const hasHitl = interaction !== undefined;
  const timingRange = state.startedAt !== undefined || state.finishedAt !== undefined
    ? [
      state.startedAt !== undefined
        ? formatRunClock(state.startedAt, locale)
        : "—",
      state.finishedAt !== undefined
        ? formatRunClock(state.finishedAt, locale)
        : "—",
    ].join(" — ")
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

  function renderSessionDock(dockTone: SessionDockTone): ReactNode {
    if (!canOpenConversation) {
      return null;
    }
    const hitlTone = dockTone === "hitl";
    return (
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        className={cn(
          "group/session-dock relative shrink-0 cursor-pointer rounded-full",
          "transition-[transform,background-color,border-color,box-shadow,color] duration-200 motion-reduce:transition-none",
          hitlTone
            ? cn(
              "size-8 border border-amber-500/25 bg-background/70 text-amber-950/80",
              "hover:border-amber-500/40 hover:bg-amber-500/10 hover:text-amber-950",
              "dark:text-amber-100/85 dark:hover:text-amber-50",
              isConversationOpen && "border-amber-500/45 bg-amber-500/15 text-amber-950 dark:text-amber-50",
            )
            : cn(
              "ml-auto size-9 border shadow-sm",
              isConversationOpen
                ? "border-primary/20 bg-primary/10 text-primary hover:bg-primary/15"
                : "border-border/80 bg-background hover:-translate-y-0.5 hover:border-primary/30 hover:shadow-md",
            ),
        )}
        aria-expanded={isConversationOpen}
        aria-label={isConversationOpen
          ? t("workflowRun.conversation.backToAct")
          : t("workflowRun.conversation.open")}
        title={isConversationOpen
          ? t("workflowRun.conversation.backToAct")
          : t("workflowRun.conversation.open")}
        onClick={(event) => {
          event.stopPropagation();
          setConversationOpen((open) => !open);
        }}
        onKeyDown={(event) => event.stopPropagation()}
        onPointerDown={(event) => event.stopPropagation()}
      >
        {isConversationOpen
          ? <IconArrowBackUp className="size-3.5" />
          : (
            <span className="relative flex size-5 items-center justify-center">
              <IconMessageCircle className={hitlTone ? "size-4" : "size-[18px]"} />
              {showDockSpark && (
                <span
                  className={cn(
                    "absolute rounded-full border bg-background p-0.5 shadow-sm",
                    hitlTone
                      ? "-right-1.5 -top-1.5 border-amber-500/25"
                      : "-right-2 -top-2 border-border/60",
                  )}
                >
                  <IconSparkles
                    className={cn(
                      "size-2.5 transition-transform duration-200 group-hover/session-dock:rotate-12 motion-reduce:transition-none",
                      hitlTone ? "text-amber-700 dark:text-amber-300" : "text-primary/85",
                    )}
                  />
                </span>
              )}
              {conversationMessageCount > 0 && (
                <span
                  className={cn(
                    "absolute flex min-w-4 items-center justify-center rounded-full px-1 text-[9px] font-semibold leading-4",
                    hitlTone
                      ? "-right-1 -top-1 border border-amber-600/20 bg-amber-700 text-amber-50 dark:bg-amber-500 dark:text-amber-950"
                      : "-right-1.5 -top-1 border border-background bg-primary text-primary-foreground",
                  )}
                >
                  {conversationMessageCount}
                </span>
              )}
            </span>
          )}
      </Button>
    );
  }

  const stageDock = renderSessionDock("stage");
  const hitlDock = renderSessionDock("hitl");

  const hitlFooter = (() => {
    if (!hasHitl) {
      return undefined;
    }
    const accessory = hitlDock;
    const body = typeof interaction === "function"
      ? interaction({ accessory })
      : (
        <div className="overflow-hidden rounded-xl border border-amber-500/25 bg-amber-500/[0.04]">
          <div className="flex items-center justify-between gap-2 border-b border-amber-500/15 px-2.5 py-1.5">
            <p className="truncate text-[11px] font-medium text-amber-950/75 dark:text-amber-100/75">
              {t("workflowRun.hitl.panelLabel")}
            </p>
            {accessory}
          </div>
          <div className="p-2.5 pt-2">{interaction}</div>
        </div>
      );
    return (
      <div
        className="pt-1"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => event.stopPropagation()}
        onPointerDown={(event) => event.stopPropagation()}
      >
        {body}
      </div>
    );
  })();

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
          && "cursor-pointer hover:border-foreground/25 hover:shadow-sm",
        // Scale only when the whole card is the hit target —not when HITL
        // lives in the footer (CSS :active would otherwise shake the card
        // while pressing the composer).
        interactive && interaction === undefined && "active:scale-[0.99]",
        emphasized && live && state.status === "running" && "theater-live-breathe",
        emphasized
          && live
          && state.status === "awaiting_input"
          && "theater-live-breathe-amber",
      )}
      ariaLabel={interactive
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
      body={isConversationOpen
        ? (
          <p className="mt-1 text-[11px] text-muted-foreground">
            {t("workflowRun.conversation.sessionMode")}
          </p>
        )
        : compact
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
      details={isConversationOpen
        ? (
          <div
            className="px-3 pb-1 pt-2 animate-in fade-in slide-in-from-bottom-1 duration-200 motion-reduce:animate-none"
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            <RunNodeConversation
              input={state.input}
              conversation={conversation}
              status={state.status}
            />
          </div>
        )
        : undefined}
      footer={hitlFooter !== undefined
        ? hitlFooter
        : isConversationOpen
        ? <div className="flex items-center">{stageDock}</div>
        : (
          <div className="flex items-end gap-3">
            <div className="min-w-0 flex-1">{metrics}</div>
            {stageDock}
          </div>
        )}
      onClick={interactive ? onSelect : undefined}
      onKeyDown={interactive
        ? (event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onSelect?.();
          }
        }
        : undefined}
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
    />
  );
}
