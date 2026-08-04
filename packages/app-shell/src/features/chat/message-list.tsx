import { useEffect, useMemo, useRef, useState } from "react";
import { AgentActivityDots } from "../../components/agent-activity-dots";
import { useTranslation } from "react-i18next";
import { AnchorHighlight } from "./anchor-highlight";
import { ConversationNavigator } from "./conversation-navigator";
import { useConversationNavigation } from "./conversation-navigation";
import { MessageBubble } from "./message-bubble";
import { ResponseTurn } from "./response-turn";
import type { ChatTurn } from "@ora/chat";

interface MessageListProps {
  turns: ChatTurn[];
  userName: string;
  isResponding: boolean;
}

/** The scrollable turn thread, kept pinned to live ACP activity unless the reader scrolls away. */
export function MessageList({ turns, userName, isResponding }: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastTurn = turns.at(-1);
  const lastAnchorId = lastTurn === undefined
    ? null
    : `${lastTurn.id}:${lastTurn.items.length === 0 && lastTurn.status === "streaming" ? "user" : "response"}`;
  const lastItem = lastTurn?.items.at(-1);
  const lastUserMessageId = lastTurn?.userMessage.id;
  // Hide the running indicator while the answer itself is streaming: the growing
  // text already shows the agent is live, so a second "working" line under it
  // just reads as noise. It returns for thoughts, tool calls, and the waits between.
  const streamingBody = lastItem?.kind === "message" && lastItem.role === "assistant";
  const showRunning = isResponding && !streamingBody;
  const navigation = useConversationNavigation({
    scrollRef,
    contentRef,
    followTailKey: `${turns.length}:${lastUserMessageId ?? ""}`,
    lastAnchorId,
  });

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={scrollRef}
        onScroll={navigation.handleScroll}
        onWheel={(event) => navigation.handleWheel(event.deltaY)}
        onPointerDown={navigation.beginPointerScroll}
        onPointerUp={navigation.endPointerScroll}
        onPointerCancel={navigation.endPointerScroll}
        onTouchStart={navigation.beginPointerScroll}
        onTouchEnd={navigation.endPointerScroll}
        onTouchCancel={navigation.endPointerScroll}
        data-testid="message-list"
        aria-live="polite"
        className="scrollbar-hide h-full min-h-0 animate-in overflow-y-auto fade-in duration-500"
      >
        <div ref={contentRef} className="mx-auto w-full max-w-[760px] px-3 pb-4 pt-5 sm:px-5 sm:pt-8">
          {turns.map((turn) => (
            <div key={turn.id} data-turn-anchor={turn.id}>
              <div data-turn-user data-conversation-anchor={`${turn.id}:user`}>
                <MessageBubble message={turn.userMessage} userName={userName} />
              </div>
              {(turn.items.length > 0 || turn.status !== "streaming") && (
                <div data-turn-response data-conversation-anchor={`${turn.id}:response`} className="relative overflow-visible rounded-xl">
                  <AnchorHighlight />
                  <ResponseTurn turn={turn} userName={userName} />
                </div>
              )}
            </div>
          ))}
          {showRunning && <RunningIndicator />}
          <div className="h-8" />
        </div>
      </div>
      <ConversationNavigator
        turns={turns}
        activeAnchorId={navigation.activeAnchorId}
        isAtTail={navigation.isAtTail}
        onNavigate={navigation.navigateToAnchor}
        onNavigateToTail={navigation.navigateToTail}
      />
    </div>
  );
}

/** Word rotation cadence — slow enough to read each phrase, quick enough to feel alive. */
const RUNNING_WORD_INTERVAL_MS = 2600;

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
    () => t("chat.runningWords").split("|").map((word) => word.trim()).filter(Boolean),
    [t],
  );
  const [index, setIndex] = useState(0);

  useEffect(() => {
    if (words.length <= 1 || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const timer = setInterval(() => setIndex((current) => (current + 1) % words.length), RUNNING_WORD_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [words]);

  const word = words[index % words.length] ?? words[0] ?? "";
  return (
    <div className="flex items-center gap-3 py-4" role="status" aria-label={t("chat.typing")}>
      <span className="flex size-6 shrink-0 items-center justify-center text-muted-foreground">
        <AgentActivityDots label={t("common.running")} dotClassName="size-[3.5px]" />
      </span>
      {/* Keyed so each phrase crossfades in as the rotation advances. */}
      <span key={word} className="animate-in text-sm text-muted-foreground fade-in duration-500">{word}</span>
    </div>
  );
}
