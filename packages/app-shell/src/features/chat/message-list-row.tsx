import { memo, useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AgentActivityDots } from "../../components/agent-activity-dots";
import { AnchorHighlight } from "./anchor-highlight";
import { NonTextItemView } from "./activity-phase";
import {
  ChatLinkContext,
  type ChatLinkContextValue,
} from "./chat-link/context";
import { MessageBubble } from "./message-bubble";
import { DisplayTurnItemView, TurnEnding } from "./response-turn";
import { TurnDiffSummary } from "./turn-diff-summary";
import { buildTurnDisplayItems } from "./turn-item-grouping";
import type { ChatTurn } from "@ora/chat";
import type { MessageListRow } from "./message-list-rows";

/** Word rotation cadence — slow enough to read each phrase, quick enough to feel alive. */
const RUNNING_WORD_INTERVAL_MS = 5000;
/** Jitter applied to each rotation so the cadence doesn't feel metronomic (golden ratio, in ms). */
const RUNNING_WORD_JITTER_MS = 618;

interface MessageListRowViewProps {
  row: MessageListRow;
  turns: ChatTurn[];
  userName: string;
  chatLinkForTurn: (turnIndex: number) => ChatLinkContextValue | null;
}

/** One independently measurable thread row (prompt, activity, reply, or chrome). */
export const MessageListRowView = memo(function MessageListRowView({
  row,
  turns,
  userName,
  chatLinkForTurn,
}: MessageListRowViewProps) {
  switch (row.type) {
    case "modelChange":
      return <ModelChangeDivider modelName={row.modelName} />;
    case "user": {
      const turn = turns[row.turnIndex];
      if (turn === undefined) {
        return null;
      }
      return (
        <div data-turn-anchor={turn.id}>
          <div data-turn-user data-conversation-anchor={`${turn.id}:user`}>
            <MessageBubble message={turn.userMessage} userName={userName} />
          </div>
        </div>
      );
    }
    case "display": {
      const turn = turns[row.turnIndex];
      if (turn === undefined) {
        return null;
      }
      const displayItems = buildTurnDisplayItems(turn.items, turn.status);
      const item = displayItems[row.displayIndex];
      if (item === undefined) {
        return null;
      }
      return (
        <ResponseRow
          turn={turn}
          chatLink={chatLinkForTurn(row.turnIndex)}
          responseAnchor={row.responseAnchor}
        >
          <DisplayTurnItemView
            item={item}
            turn={turn}
            userName={userName}
            displayIndex={row.displayIndex}
            displayCount={displayItems.length}
          />
        </ResponseRow>
      );
    }
    case "activityAtom": {
      const turn = turns[row.turnIndex];
      if (turn === undefined) {
        return null;
      }
      const displayItems = buildTurnDisplayItems(turn.items, turn.status);
      const phase = displayItems[row.displayIndex];
      if (phase?.kind !== "activityPhase") {
        return null;
      }
      const atom = phase.items[row.atomIndex];
      if (atom === undefined) {
        return null;
      }
      const lastAtom = row.atomIndex === phase.items.length - 1;
      const lastPhase = row.displayIndex === displayItems.length - 1;
      return (
        <ResponseRow
          turn={turn}
          chatLink={chatLinkForTurn(row.turnIndex)}
          responseAnchor={row.responseAnchor}
        >
          <NonTextItemView
            item={atom}
            turnStatus={turn.status}
            isLatestActivity={lastPhase && lastAtom}
          />
        </ResponseRow>
      );
    }
    case "turnMeta": {
      const turn = turns[row.turnIndex];
      if (turn === undefined) {
        return null;
      }
      return (
        <ResponseRow
          turn={turn}
          chatLink={chatLinkForTurn(row.turnIndex)}
          responseAnchor={row.responseAnchor}
        >
          <TurnEnding turn={turn} />
          <TurnDiffSummary turn={turn} />
        </ResponseRow>
      );
    }
    case "running":
      return <RunningIndicator />;
    case "pad":
      return <div className="h-8" />;
  }
});

function ResponseRow({
  turn,
  chatLink,
  responseAnchor,
  children,
}: {
  turn: ChatTurn;
  chatLink: ChatLinkContextValue | null;
  responseAnchor: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const framed = responseAnchor ? (
    <section
      aria-label={t("chat.assistantReplied")}
      data-turn-response
      data-conversation-anchor={`${turn.id}:response`}
      className="relative overflow-visible rounded-xl py-1.5"
    >
      <AnchorHighlight />
      <div className="min-w-0 space-y-2.5">{children}</div>
    </section>
  ) : (
    <div className="min-w-0 space-y-2.5 py-1.5">{children}</div>
  );
  return (
    <ChatLinkContext.Provider value={chatLink}>
      {framed}
    </ChatLinkContext.Provider>
  );
}

/**
 * Marks where the answering model changed, so replies above and below a divider
 * are not mistaken for the work of one model.
 */
function ModelChangeDivider({ modelName }: { modelName: string }) {
  const { t } = useTranslation();
  return (
    <div
      role="separator"
      aria-label={t("chat.modelChange", { model: modelName })}
      className="flex items-center gap-3 py-4"
    >
      <span className="h-px flex-1 bg-border" />
      <span className="whitespace-nowrap text-xs text-muted-foreground">
        {t("chat.modelChange", { model: modelName })}
      </span>
      <span className="h-px flex-1 bg-border" />
    </div>
  );
}

/**
 * A playful "still working" line pinned to the foot of the live turn.
 *
 * Unlike the old typing dots, this stays for the whole response — through every
 * tool call and the quiet gaps between them — so the thread never looks frozen
 * while the agent is busy. The nine-dot grid carries the motion; the rotating
 * phrase reassures that time is passing rather than that anything has stalled.
 */
function RunningIndicator() {
  const { t } = useTranslation();
  const words = useMemo(
    () =>
      t("chat.runningWords")
        .split("|")
        .map((word) => word.trim())
        .filter(Boolean),
    [t],
  );
  const [index, setIndex] = useState(0);

  useEffect(() => {
    if (
      words.length <= 1 ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    )
      return;
    let timer: ReturnType<typeof setTimeout>;
    const scheduleNext = () => {
      const delay =
        RUNNING_WORD_INTERVAL_MS +
        (Math.random() * 2 - 1) * RUNNING_WORD_JITTER_MS;
      timer = setTimeout(() => {
        setIndex((current) => (current + 1) % words.length);
        scheduleNext();
      }, delay);
    };
    scheduleNext();
    return () => clearTimeout(timer);
  }, [words]);

  const word = words[index % words.length] ?? words[0] ?? "";
  return (
    <div
      className="flex items-center gap-3 py-4"
      role="status"
      aria-label={t("chat.typing")}
    >
      <span className="flex size-6 shrink-0 items-center justify-center text-muted-foreground">
        <AgentActivityDots
          label={t("common.running")}
          dotClassName="size-[3.5px]"
        />
      </span>
      {/* Keyed so each phrase crossfades in as the rotation advances. */}
      <span
        key={word}
        className="animate-in text-sm text-muted-foreground fade-in duration-500"
      >
        {word}
      </span>
    </div>
  );
}
