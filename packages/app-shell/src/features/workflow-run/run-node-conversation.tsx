import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  cn,
} from "@ora/ui";
import {
  IconChevronDown,
  IconRoute,
  IconTimeline,
  IconTool,
} from "@tabler/icons-react";
import type { ChatMessage } from "@ora/chat";
import { ConversationNavigator } from "../chat/conversation-navigator";
import {
  useConversationNavigation,
  type ConversationAnchor,
} from "../chat/conversation-navigation";
import { AnchorHighlight } from "../chat/anchor-highlight";
import { MessageBubble } from "../chat/message-bubble";
import { chainWheelToScrollParent } from "../chat/scroll-chain";
import type {
  GraphWorkflowNodeIo,
  GraphWorkflowNodeStatus,
  WorkflowNodeConversationActivity,
  WorkflowNodeConversationItem,
  WorkflowNodeConversationMessage,
} from "@ora/workflow-runtime";

interface RunNodeConversationProps {
  input?: GraphWorkflowNodeIo;
  conversation: WorkflowNodeConversationItem[];
  status: GraphWorkflowNodeStatus;
}

type DisplayItem =
  | WorkflowNodeConversationMessage
  | { kind: "activity_group"; id: string; items: WorkflowNodeConversationActivity[] };

/**
 * Read-only node conversation that reuses the full chat bubble and Markdown
 * renderer while keeping thoughts and tool calls behind one small disclosure.
 */
export function RunNodeConversation({
  input,
  conversation,
  status,
}: RunNodeConversationProps) {
  const { t } = useTranslation();
  const waitingForReply = status === "running" || status === "awaiting_input";
  const displayItems = useMemo(
    () => buildDisplayItems(conversation, input),
    [conversation, input],
  );
  const anchors = useMemo(
    () => buildNodeAnchors(displayItems, t),
    [displayItems, t],
  );
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastUserMessageId = useMemo(() => {
    for (let index = displayItems.length - 1; index >= 0; index -= 1) {
      const item = displayItems[index];
      if (item.kind === "message" && item.role === "user") {
        return item.id;
      }
    }
    return undefined;
  }, [displayItems]);
  const navigation = useConversationNavigation({
    scrollRef,
    contentRef,
    followTailKey: `${displayItems.length}:${lastUserMessageId ?? ""}`,
    lastAnchorId: anchors.at(-1)?.id ?? null,
  });

  return (
    <div aria-label={t("workflowRun.conversation.label")} data-node-conversation>
      <div className="relative">
        <div
          ref={scrollRef}
          onScroll={navigation.handleScroll}
          onWheel={(event) => {
            navigation.handleWheel(event.deltaY);
            // At the conversation edge, keep scrolling the Theater stage so
            // HITL / footer below can come into view without moving the cursor.
            chainWheelToScrollParent(event, event.currentTarget);
          }}
          onPointerDown={navigation.beginPointerScroll}
          onPointerUp={navigation.endPointerScroll}
          onPointerCancel={navigation.endPointerScroll}
          onTouchStart={navigation.beginPointerScroll}
          onTouchEnd={navigation.endPointerScroll}
          onTouchCancel={navigation.endPointerScroll}
          data-testid="node-conversation-scroll"
          aria-live="polite"
          className="scrollbar-hide max-h-[28rem] overflow-y-auto overscroll-contain"
        >
          <div ref={contentRef} className="space-y-1 px-1 py-1">
            {displayItems.length > 0
              ? displayItems.map((item) => item.kind === "activity_group"
                ? <CollapsedActivityGroup key={item.id} items={item.items} />
                : (
                  <div
                    key={item.id}
                    data-conversation-anchor={item.id}
                    className={item.role === "assistant"
                      ? "relative overflow-visible rounded-xl"
                      : undefined}
                  >
                    {item.role === "assistant" ? <AnchorHighlight /> : null}
                    <MessageBubble
                      message={toChatMessage(item)}
                      userName={t("account.unknownIdentity")}
                      compact
                      showAnchorHighlight={item.role === "user"}
                      streaming={item.status === "streaming"}
                    />
                  </div>
                ))
              : (
                <div className="rounded-xl border border-dashed border-border/80 px-3 py-5 text-center">
                  <p className="text-xs font-medium text-foreground/80">
                    {waitingForReply
                      ? t("workflowRun.conversation.waiting")
                      : t("workflowRun.conversation.empty")}
                  </p>
                </div>
              )}
          </div>
        </div>
        <ConversationNavigator
          anchors={anchors}
          activeAnchorId={navigation.activeAnchorId}
          isAtTail={navigation.isAtTail}
          onNavigate={navigation.navigateToAnchor}
          onNavigateToTail={navigation.navigateToTail}
          placement="container"
          minAnchors={3}
        />
      </div>
    </div>
  );
}

/** Builds stable anchors from visible node messages while leaving folded activity out of the track. */
function buildNodeAnchors(
  items: DisplayItem[],
  t: ReturnType<typeof useTranslation>["t"],
): ConversationAnchor[] {
  let userIndex = 0;
  let agentIndex = 0;
  return items.flatMap((item) => {
    if (item.kind === "activity_group") return [];
    const isUser = item.role === "user";
    const index = isUser ? ++userIndex : ++agentIndex;
    const preview = item.markdown.trim() || t("workflowRun.conversation.empty");
    return [{
      id: item.id,
      label: t(
        isUser
          ? "workflowRun.conversation.userAnchorLabel"
          : "workflowRun.conversation.agentAnchorLabel",
        { index },
      ),
      preview,
      summary: preview.replace(/\s+/g, " ").trim(),
      role: item.role,
    }];
  });
}

/** Keeps activity chronology while allowing consecutive secondary items to collapse together. */
function buildDisplayItems(
  conversation: WorkflowNodeConversationItem[],
  input?: GraphWorkflowNodeIo,
): DisplayItem[] {
  const source = conversation.length > 0
    ? conversation
    : fallbackInput(input);
  const result: DisplayItem[] = [];
  let activities: WorkflowNodeConversationActivity[] = [];

  const flushActivities = () => {
    if (activities.length > 0) {
      result.push({
        kind: "activity_group",
        id: `activity-group-${activities[0].id}`,
        items: activities,
      });
      activities = [];
    }
  };

  for (const item of source) {
    if (item.kind === "activity") {
      activities.push(item);
    } else {
      flushActivities();
      result.push(item);
    }
  }
  flushActivities();
  return result;
}

/** Shows node input during the short hand-off before the first stream item arrives. */
function fallbackInput(input?: GraphWorkflowNodeIo): WorkflowNodeConversationMessage[] {
  const content = input?.detail?.trim() || input?.summary.trim();
  if (content === undefined || content === "") {
    return [];
  }
  const now = new Date().toISOString();
  return [{
    kind: "message",
    id: "node-input-preview",
    runId: "preview",
    nodeId: "preview",
    sessionId: "preview",
    role: "user",
    markdown: content,
    status: "complete",
    createdAt: now,
    updatedAt: now,
  }];
}

/** Adapts the transport-neutral message to the exact shape used by the full chat bubble. */
function toChatMessage(message: WorkflowNodeConversationMessage): ChatMessage {
  const createdAt = Date.parse(message.createdAt);
  return {
    kind: "message",
    id: message.id,
    role: message.role,
    content: message.markdown,
    createdAt: Number.isFinite(createdAt) ? createdAt : Date.now(),
  };
}

/** Collapses thoughts and tool calls into the same secondary timeline language as full chat. */
function CollapsedActivityGroup({
  items,
}: {
  items: WorkflowNodeConversationActivity[];
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const preview = items.map((item) => item.summary).join(" · ");

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className="overflow-hidden rounded-lg border-l-2 border-border/80"
    >
      <CollapsibleTrigger className="flex min-h-9 w-full items-center gap-2 rounded-r-md px-2.5 py-1.5 text-left text-xs text-muted-foreground outline-none transition-colors hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50">
        <IconTimeline className="size-3.5 shrink-0 text-sky-700 dark:text-sky-400" />
        <span className="min-w-0 flex-1 truncate">
          <span className="font-medium text-foreground/80">
            {t("workflowRun.conversation.hiddenActivity", { count: items.length })}
          </span>
          <span className="ml-1.5 opacity-70">{preview}</span>
        </span>
        <IconChevronDown
          className={cn(
            "size-3.5 shrink-0 transition-transform duration-200 motion-reduce:transition-none",
            open && "rotate-180",
          )}
        />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="space-y-1 border-t border-border/60 bg-muted/15 px-2.5 py-2">
          {items.map((item) => (
            <div key={item.id} className="flex gap-2 rounded-md px-1 py-1.5 text-[11px] text-muted-foreground">
              {item.activityKind === "thought"
                ? <IconRoute className="mt-0.5 size-3.5 shrink-0 text-violet-600 dark:text-violet-400" />
                : <IconTool className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />}
              <div className="min-w-0">
                <p className="font-medium text-foreground/75">{item.summary}</p>
                {item.detail !== undefined && (
                  <p data-selectable className="mt-0.5 whitespace-pre-wrap leading-5">
                    {item.detail}
                  </p>
                )}
              </div>
            </div>
          ))}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
