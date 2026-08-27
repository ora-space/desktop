import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  IconLayoutSidebarRightCollapse,
  IconLayoutSidebarRightExpand,
} from "@tabler/icons-react";
import { Badge, Button, cn } from "@ora/ui";
import { createMockWorkflowNodeType } from "@ora/workflow-mock";
import { formatRunClock } from "../../lib/format";
import { useAgentCatalog } from "../chat/agent-catalog";
import {
  AgentExecutionModeMark,
  WorkflowNodeCardShell,
} from "../workflow-node-chrome";
import {
  resolveTheaterActDetail,
  resolveTheaterActInstruction,
} from "./agent-config-display";
import {
  RunActSessionDock,
  type ActSessionDockTone,
} from "./run-act-session-dock";
import { RunBriefPopover } from "./run-brief-popover";
import { RunStatusBadge } from "./run-status-mark";
import { isNodeWorking, runStatusTone } from "./run-status-style";
import { RunNodeSessionChat } from "./run-node-session-chat";
import { shouldPreviewBrief } from "./should-preview-brief";
import type {
  GraphWorkflowNodeState,
  WorkflowNodeConversationItem,
  WorkflowNodeData,
} from "@ora/workflow-runtime";
import "./theater-motion.css";

interface RunTheaterActCardProps {
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  /** Run identifier used by an interactive node's explicit completion action. */
  runId?: string;
  /** Node identifier used by an interactive node's explicit completion action. */
  nodeId?: string;
  /** Soft emphasis when this act is live (running / awaiting). */
  live: boolean;
  /** Glanceable outcome count; detail lives in the act inspector. */
  artifactCount?: number;
  /** Large primary stage vs secondary parallel card. */
  variant?: "stage" | "compact";
  /** Whether the companion inspector is currently expanded. */
  inspectorOpen?: boolean;
  /** Toggles the companion inspector from the dedicated header control. */
  onToggleInspector?: () => void;
  /** Stronger stage presence when this card is the focused parallel act. */
  emphasized?: boolean;
  /**
   * HITL surface for this act. Prefer the render form so the session dock can
   * live inside HITL chrome; a plain node falls back to a shared action strip.
   */
  interaction?:
    ReactNode | ((slots: { accessory: ReactNode | null }) => ReactNode);
  /** Filtered node session items; secondary activity is disclosed in-place. */
  conversation?: WorkflowNodeConversationItem[];
  /** Parallel peers opt in only for the focused card to keep carousel gestures stable. */
  conversationEnabled?: boolean;
  /** Controlled session-dock open state (workspace lifts this across view remounts). */
  conversationOpen?: boolean;
  onConversationOpenChange?: (open: boolean) => void;
  /** Lets the workspace move an open session dock after interactive completion. */
  onNodeCompleted?: (nodeId: string) => void;
}

/**
 * Theater act card: instruction + metrics on the stage surface.
 * A dedicated header control opens the companion inspector so nested session
 * interactions never compete with a whole-card click target.
 */
export function RunTheaterActCard({
  data,
  state,
  runId,
  nodeId,
  live,
  artifactCount = 0,
  variant = "stage",
  inspectorOpen = false,
  onToggleInspector,
  emphasized = true,
  interaction,
  conversation = [],
  conversationEnabled = true,
  conversationOpen: conversationOpenProp,
  onConversationOpenChange,
  onNodeCompleted,
}: RunTheaterActCardProps) {
  const { i18n, t } = useTranslation();
  const locale =
    i18n.resolvedLanguage === "en-US" ? ("en-US" as const) : ("zh-CN" as const);
  const agentCatalog = useAgentCatalog();
  const kindLabel = createMockWorkflowNodeType(data.kind, locale).label;
  const tone = runStatusTone(state.status);
  const detail = resolveTheaterActDetail(data, agentCatalog);
  const instruction = resolveTheaterActInstruction(data);
  const compact = variant === "compact";
  const [uncontrolledConversationOpen, setUncontrolledConversationOpen] =
    useState(false);
  const conversationOpen = conversationOpenProp ?? uncontrolledConversationOpen;
  const setConversationOpen =
    onConversationOpenChange ?? setUncontrolledConversationOpen;
  const conversationMessageCount = useMemo(
    () =>
      conversation.reduce(
        (count, item) => (item.kind === "message" ? count + 1 : count),
        0,
      ),
    [conversation],
  );
  // Keep the session dock available during HITL so readers can inspect prior
  // node messages before answering a permission or clarify gate.
  const canUseConversation = !compact && conversationEnabled;
  const isConversationOpen = conversationOpen && canUseConversation;
  const sessionChatIdentity =
    isConversationOpen && state.sessionId != null
      ? { sessionId: state.sessionId }
      : null;
  const sessionInteraction =
    sessionChatIdentity !== null &&
    data.agentConfig?.interactive === true &&
    runId !== undefined &&
    nodeId !== undefined
      ? { runId, nodeId }
      : undefined;
  const hasHitl = interaction !== undefined;
  const timingRange =
    state.startedAt !== undefined || state.finishedAt !== undefined
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
    </div>
  );

  function renderSessionDock(dockTone: ActSessionDockTone): ReactNode {
    if (!canUseConversation) {
      return null;
    }
    return (
      <RunActSessionDock
        open={isConversationOpen}
        messageCount={conversationMessageCount}
        onOpenChange={setConversationOpen}
        tone={dockTone}
      />
    );
  }

  const stageDock = renderSessionDock("stage");
  const hitlDock = renderSessionDock("hitl");

  const hitlFooter = (() => {
    if (!hasHitl) {
      return undefined;
    }
    const accessory = hitlDock;
    const body =
      typeof interaction === "function" ? (
        interaction({ accessory })
      ) : (
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
      titleAccessory={
        data.kind === "agent" ? (
          <AgentExecutionModeMark
            interactive={data.agentConfig?.interactive === true}
          />
        ) : undefined
      }
      className={cn(
        compact
          ? "w-full"
          : isConversationOpen
            ? "flex h-full min-h-0 w-full max-w-none flex-col overflow-hidden"
            : "mx-auto w-full max-w-xl",
        "transition-[border-color,box-shadow] duration-200 ease-out motion-reduce:transition-none",
        emphasized &&
          live &&
          state.status === "running" &&
          "theater-live-breathe",
        emphasized &&
          live &&
          state.status === "awaiting_input" &&
          "theater-live-breathe-amber",
      )}
      ariaLabel={`${data.title}: ${t(tone.labelKey)}`}
      aria-live={compact ? undefined : "polite"}
      frameClassName={cn(
        tone.ring,
        "ring-1 transition-[box-shadow,ring-color] duration-300",
        live && state.status === "running" && "ring-sky-500/35",
        live && state.status === "awaiting_input" && "ring-amber-500/35",
      )}
      detailsClassName={
        isConversationOpen ? "min-h-0 flex-1 overflow-hidden" : undefined
      }
      headerAccessory={
        <div className="flex items-center gap-1.5">
          {artifactCount > 0 && (
            <Badge variant="secondary" className="tabular-nums text-[10px]">
              {t("workflowRun.artifacts.countBadge", { count: artifactCount })}
            </Badge>
          )}
          <RunStatusBadge
            status={state.status}
            live={emphasized && isNodeWorking(state.status)}
          />
          {state.stopReason != null && (
            <span className="text-[10px] text-muted-foreground">
              {state.stopReason}
            </span>
          )}
        </div>
      }
      headerEnd={
        onToggleInspector !== undefined ? (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className={cn(
              "size-9 shrink-0 rounded-full border shadow-sm",
              inspectorOpen
                ? "border-primary/40 bg-primary/10 text-primary hover:bg-primary/15"
                : "border-border/80 bg-background hover:border-primary/30 hover:bg-primary/5",
            )}
            aria-label={t(
              inspectorOpen
                ? "workflowRun.inspector.collapse"
                : "workflowRun.theater.openInspector",
            )}
            title={t(
              inspectorOpen
                ? "workflowRun.inspector.collapse"
                : "workflowRun.theater.openInspector",
            )}
            aria-pressed={inspectorOpen}
            onPointerDown={(event) => event.stopPropagation()}
            onClick={onToggleInspector}
          >
            {inspectorOpen ? (
              <IconLayoutSidebarRightCollapse className="size-4" />
            ) : (
              <IconLayoutSidebarRightExpand className="size-4" />
            )}
          </Button>
        ) : undefined
      }
      body={
        isConversationOpen ? (
          <p className="mt-1 text-[11px] text-muted-foreground">
            {t("workflowRun.conversation.sessionMode")}
          </p>
        ) : compact ? (
          <p className="mt-1 line-clamp-2 text-[11px] leading-4 text-muted-foreground">
            {data.description}
          </p>
        ) : (
          <>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              {data.description}
            </p>
            <div className="mt-5 rounded-xl border border-border/80 bg-muted/30 px-4 py-3">
              <p className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
                {t("workflowRun.theater.instruction")}
              </p>
              {shouldPreviewBrief(instruction) ? (
                <div className="mt-1.5">
                  <RunBriefPopover
                    title={t("workflowRun.theater.instruction")}
                    body={instruction}
                    openLabel={t("workflowRun.inspector.textOpen", {
                      field: t("workflowRun.theater.instruction"),
                    })}
                    side="bottom"
                    stopPropagation
                    className="border-border/60 bg-background/70 hover:bg-background/90"
                  >
                    <span className="line-clamp-4 whitespace-pre-wrap text-sm leading-6 text-foreground/90">
                      {instruction}
                    </span>
                  </RunBriefPopover>
                </div>
              ) : (
                <p className="mt-1.5 line-clamp-4 text-sm leading-6 text-foreground/90">
                  {instruction === "" ? "—" : instruction}
                </p>
              )}
              {detail !== undefined && (
                <p className="mt-2 font-mono text-[11px] text-muted-foreground">
                  {detail}
                </p>
              )}
            </div>
          </>
        )
      }
      details={
        isConversationOpen ? (
          <div
            className="flex h-full min-h-0 flex-col px-3 pb-1 pt-2 animate-in fade-in slide-in-from-bottom-1 duration-200 motion-reduce:animate-none"
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            {sessionChatIdentity !== null ? (
              <RunNodeSessionChat
                sessionId={sessionChatIdentity.sessionId}
                status={state.status}
                interaction={sessionInteraction}
                onNodeCompleted={onNodeCompleted}
                sessionActions={
                  sessionInteraction !== undefined || hitlFooter === undefined
                    ? stageDock
                    : undefined
                }
              />
            ) : (
              <div className="flex min-h-0 flex-1 items-center justify-center text-sm text-muted-foreground">
                {t("workflowRun.conversation.waiting")}
              </div>
            )}
          </div>
        ) : undefined
      }
      footer={
        sessionInteraction !== undefined ? undefined : hitlFooter !==
          undefined ? (
          hitlFooter
        ) : sessionChatIdentity !== null ? undefined : isConversationOpen ? (
          <div className="flex items-center">{stageDock}</div>
        ) : (
          <div className="flex items-end gap-3">
            <div className="min-w-0 flex-1">{metrics}</div>
            {stageDock}
          </div>
        )
      }
    />
  );
}
