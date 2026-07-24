import { IconAlertTriangle, IconBan, IconInfoCircle } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import type { ChatToolCall, ChatTurn, ChatTurnItem } from "@ora/chat";
import { OraMark } from "../../components/ora-mark";
import { MessageBubble } from "./message-bubble";
import { PlanBlock } from "./plan-block";
import { ThoughtBlock } from "./thought-block";
import { ToolCallBlock } from "./tool-call-block";
import { ToolCallGroup } from "./tool-call-group";
import { toolCallGroupKind, type ToolCallGroupKind } from "./tool-call-group-kind";

interface ToolGroup {
  kind: "toolGroup";
  id: string;
  groupKind: ToolCallGroupKind;
  tools: ChatToolCall[];
}

type DisplayTurnItem = ChatTurnItem | ToolGroup;

interface ResponseTurnProps {
  turn: ChatTurn;
  userName: string;
}

/** Groups all agent activity for one prompt under a single assistant identity. */
export function ResponseTurn({ turn, userName }: ResponseTurnProps) {
  const { t } = useTranslation();
  const displayItems = groupAdjacentTools(turn.items);
  return (
    <section className="flex gap-3 py-3" aria-label={t("chat.assistantReplied")}>
      <OraMark size="sm" />
      <div className="min-w-0 flex-1 space-y-2.5">
        {displayItems.map((item, index) => {
          switch (item.kind) {
            case "thought":
              return <ThoughtBlock key={item.id} thought={item} hasFollowingActivity={index < displayItems.length - 1} />;
            case "plan":
              return <PlanBlock key={item.id} plan={item} />;
            case "toolCall":
              return <ToolCallBlock key={item.id} tool={item} />;
            case "toolGroup":
              return <ToolCallGroup key={item.id} kind={item.groupKind} tools={item.tools} />;
            case "message":
              return <MessageBubble key={item.id} message={item} userName={userName} embeddedAssistant />;
            case "unsupportedContent":
              return (
                <p key={item.id} className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                  {t("chat.unsupportedContent", { type: item.contentType })}
                </p>
              );
          }
        })}
        <TurnEnding turn={turn} />
      </div>
    </section>
  );
}

/** Groups adjacent tools by intent while preserving boundaries created by messages and plans. */
function groupAdjacentTools(items: ChatTurnItem[]): DisplayTurnItem[] {
  const grouped: DisplayTurnItem[] = [];
  let tools: ChatToolCall[] = [];
  let groupKind: ToolCallGroupKind | null = null;

  const flushTools = () => {
    if (tools.length === 1) grouped.push(tools[0]);
    if (tools.length > 1 && groupKind !== null) {
      grouped.push({ kind: "toolGroup", id: `${groupKind}-group-${tools[0].id}`, groupKind, tools });
    }
    tools = [];
    groupKind = null;
  };

  for (const item of items) {
    const nextGroupKind = item.kind === "toolCall" ? toolCallGroupKind(item) : null;
    if (item.kind === "toolCall" && nextGroupKind !== null) {
      if (groupKind !== null && groupKind !== nextGroupKind) flushTools();
      groupKind = nextGroupKind;
      tools.push(item);
      continue;
    }
    flushTools();
    grouped.push(item);
  }
  flushTools();
  return grouped;
}

/** Explains non-standard turn endings without treating them as transport failures. */
function TurnEnding({ turn }: { turn: ChatTurn }) {
  const { t } = useTranslation();
  if (turn.status === "cancelled") {
    return <p className="flex items-center gap-1.5 text-xs text-muted-foreground"><IconBan className="size-3.5" />{t("chat.turnCancelled")}</p>;
  }
  if (turn.status === "failed") {
    return <p data-selectable className="flex items-center gap-1.5 text-xs text-destructive"><IconAlertTriangle className="size-3.5" />{turn.error ?? t("chat.turnFailed")}</p>;
  }
  if (turn.stopReason === "max_tokens" || turn.stopReason === "max_turn_requests") {
    return <p className="flex items-center gap-1.5 text-xs text-amber-700 dark:text-amber-400"><IconAlertTriangle className="size-3.5" />{t("chat.turnIncomplete")}</p>;
  }
  if (turn.stopReason === "refusal") {
    return <p className="flex items-center gap-1.5 text-xs text-muted-foreground"><IconInfoCircle className="size-3.5" />{t("chat.turnRefused")}</p>;
  }
  return null;
}
