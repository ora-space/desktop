import { useEffect, useMemo, useRef, useState } from "react";
import { AgentActivityDots } from "../../components/agent-activity-dots";
import { useTranslation } from "react-i18next";
import { AnchorHighlight } from "./anchor-highlight";
import { ConversationNavigator } from "./conversation-navigator";
import { MessageBubble } from "./message-bubble";
import { ResponseTurn } from "./response-turn";
import type { ChatTurn } from "@ora/chat";

interface MessageListProps {
  turns: ChatTurn[];
  userName: string;
  isResponding: boolean;
}

const NAVIGATION_TOP_OFFSET_PX = 12;
const TAIL_PROXIMITY_PX = 24;
const NAVIGATION_ARRIVAL_TOLERANCE_PX = 1;

interface PendingNavigation {
  scrollTop: number;
}

/** The scrollable turn thread, kept pinned to live ACP activity unless the reader scrolls away. */
export function MessageList({ turns, userName, isResponding }: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const followTailRef = useRef(true);
  const pendingNavigationRef = useRef<PendingNavigation | null>(null);
  const lastTurn = turns.at(-1);
  const lastAnchorId = lastTurn === undefined
    ? null
    : `${lastTurn.id}:${lastTurn.items.length === 0 && lastTurn.status === "streaming" ? "user" : "response"}`;
  const [navigation, setNavigation] = useState<{ activeAnchorId: string | null; lastAnchorId: string | null }>({
    activeAnchorId: lastAnchorId,
    lastAnchorId,
  });
  const activeAnchorId = navigation.lastAnchorId === lastAnchorId ? navigation.activeAnchorId : lastAnchorId;
  const lastItem = lastTurn?.items.at(-1);
  const lastUserMessageId = lastTurn?.userMessage.id;
  const tailVersion = itemVersion(lastItem);
  // Hide the running indicator while the answer itself is streaming: the growing
  // text already shows the agent is live, so a second "working" line under it
  // just reads as noise. It returns for thoughts, tool calls, and the waits between.
  const streamingBody = lastItem?.kind === "message" && lastItem.role === "assistant";
  const showRunning = isResponding && !streamingBody;

  const handleScroll = () => {
    const element = scrollRef.current;
    if (!element) return;
    followTailRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < TAIL_PROXIMITY_PX;
    const pendingNavigation = pendingNavigationRef.current;
    if (pendingNavigation) {
      const maximumScrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
      const destination = Math.min(pendingNavigation.scrollTop, maximumScrollTop);
      if (Math.abs(element.scrollTop - destination) <= NAVIGATION_ARRIVAL_TOLERANCE_PX) {
        pendingNavigationRef.current = null;
      }
      return;
    }
    const nextAnchorId = findActiveAnchorId(element);
    setNavigation((current) => (
      current.activeAnchorId === nextAnchorId && current.lastAnchorId === lastAnchorId
        ? current
        : { activeAnchorId: nextAnchorId, lastAnchorId }
    ));
  };

  /** Returns control to position-based tracking when the reader manually moves the thread. */
  const cancelPendingNavigation = () => {
    pendingNavigationRef.current = null;
  };

  useEffect(() => {
    if (lastUserMessageId === undefined) return;
    followTailRef.current = true;
  }, [turns.length, lastUserMessageId]);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element || !followTailRef.current) return;
    element.style.scrollBehavior = isResponding ? "auto" : "smooth";
    element.scrollTop = element.scrollHeight;
  }, [turns.length, lastTurn?.items.length, tailVersion, isResponding]);

  /** Moves the thread to a selected prompt or response without resuming live tail-following. */
  const navigateToAnchor = (anchorId: string) => {
    const element = scrollRef.current;
    if (!element) return;
    const anchor = Array.from(element.querySelectorAll<HTMLElement>("[data-conversation-anchor]")).find(
      (candidate) => candidate.dataset.conversationAnchor === anchorId,
    );
    if (!anchor) return;

    followTailRef.current = false;
    const top = Math.max(0, anchor.offsetTop - NAVIGATION_TOP_OFFSET_PX);
    pendingNavigationRef.current = { scrollTop: top };
    setNavigation({ activeAnchorId: anchorId, lastAnchorId });
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const behavior = reduceMotion ? "auto" : "smooth";
    if (typeof element.scrollTo === "function") element.scrollTo({ top, behavior });
    else element.scrollTop = top;
    highlightTurn(anchor, reduceMotion);
  };

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        onWheel={cancelPendingNavigation}
        onPointerDown={cancelPendingNavigation}
        onTouchStart={cancelPendingNavigation}
        data-testid="message-list"
        aria-live="polite"
        className="scrollbar-hide h-full min-h-0 animate-in overflow-y-auto fade-in duration-500"
      >
        <div className="mx-auto w-full max-w-[760px] px-3 pb-4 pt-5 sm:px-5 sm:pt-8">
          {turns.map((turn) => (
            <div key={turn.id} data-turn-anchor={turn.id}>
              <div data-turn-user data-conversation-anchor={`${turn.id}:user`}>
                <MessageBubble message={turn.userMessage} userName={userName} />
              </div>
              {(turn.items.length > 0 || turn.status !== "streaming") && (
                <div data-turn-response data-conversation-anchor={`${turn.id}:response`} className="relative rounded-xl">
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
      <ConversationNavigator turns={turns} activeAnchorId={activeAnchorId} onNavigate={navigateToAnchor} />
    </div>
  );
}

/** Briefly outlines the destination so the eye can connect the minimap action to the turn. */
function highlightTurn(anchor: HTMLElement, reduceMotion: boolean) {
  const outline = anchor.querySelector<SVGRectElement>("[data-anchor-highlight]");
  if (!outline || typeof outline.animate !== "function") return;
  if (typeof outline.getAnimations === "function") {
    outline.getAnimations().forEach((animation) => animation.cancel());
  }
  outline.animate(
    reduceMotion
      ? [
          { strokeDashoffset: 0, opacity: 0.82 },
          { strokeDashoffset: 0, opacity: 0 },
        ]
      : [
          { strokeDashoffset: 1, opacity: 0, offset: 0 },
          { strokeDashoffset: 0, opacity: 0.9, offset: 0.15 },
          { strokeDashoffset: 0, opacity: 0.9, offset: 0.75 },
          { strokeDashoffset: 0, opacity: 0, offset: 1 },
        ],
    { duration: reduceMotion ? 250 : 4000, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
  );
}

/** Finds the prompt or response aligned with the navigator's viewport-top destination. */
function findActiveAnchorId(element: HTMLDivElement): string | null {
  const anchors = Array.from(element.querySelectorAll<HTMLElement>("[data-conversation-anchor]"));
  if (anchors.length === 0) return null;
  if (element.scrollHeight - element.scrollTop - element.clientHeight < TAIL_PROXIMITY_PX) {
    return anchors.at(-1)?.dataset.conversationAnchor ?? null;
  }

  // Sharing the jump offset prevents a prompt at the top from being mistaken for its following response.
  const readingLine = element.scrollTop + NAVIGATION_TOP_OFFSET_PX;
  let activeAnchorId = anchors[0]?.dataset.conversationAnchor ?? null;
  for (const anchor of anchors) {
    if (anchor.offsetTop > readingLine) break;
    activeAnchorId = anchor.dataset.conversationAnchor ?? activeAnchorId;
  }
  return activeAnchorId;
}

/** Returns a primitive version marker for streaming content and lifecycle updates. */
function itemVersion(item: ChatTurn["items"][number] | undefined): string | number | undefined {
  if (item === undefined) return undefined;
  switch (item.kind) {
    case "message":
    case "thought":
      return item.content;
    case "plan":
    case "toolCall":
      return item.updatedAt;
    case "unsupportedContent":
      return item.id;
  }
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
    setIndex(0);
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
