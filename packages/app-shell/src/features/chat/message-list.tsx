import { useEffect, useRef } from "react";
import { OraMark } from "../../components/ora-mark";
import { useTranslation } from "react-i18next";
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
  const followTailRef = useRef(true);
  const lastTurn = turns.at(-1);
  const lastItem = lastTurn?.items.at(-1);
  const lastUserMessageId = lastTurn?.userMessage.id;
  const showTyping = isResponding && lastTurn?.items.length === 0;
  const tailVersion = itemVersion(lastItem);

  const handleScroll = () => {
    const element = scrollRef.current;
    if (!element) return;
    followTailRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
  };

  useEffect(() => {
    if (lastUserMessageId !== undefined) followTailRef.current = true;
  }, [turns.length, lastUserMessageId]);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element || !followTailRef.current) return;
    element.style.scrollBehavior = isResponding ? "auto" : "smooth";
    element.scrollTop = element.scrollHeight;
  }, [turns.length, lastTurn?.items.length, tailVersion, isResponding]);

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      data-testid="message-list"
      aria-live="polite"
      className="scrollbar-hide min-h-0 flex-1 animate-in overflow-y-auto fade-in duration-500"
    >
      <div className="mx-auto w-full max-w-[760px] px-3 pb-4 pt-5 sm:px-5 sm:pt-8">
        {turns.map((turn) => (
          <div key={turn.id}>
            <MessageBubble message={turn.userMessage} userName={userName} />
            {(turn.items.length > 0 || turn.status !== "streaming") && <ResponseTurn turn={turn} userName={userName} />}
          </div>
        ))}
        {showTyping && <TypingIndicator />}
        <div className="h-8" />
      </div>
    </div>
  );
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

/** Three pulsing dots shown before the first visible agent update. */
function TypingIndicator() {
  const { t } = useTranslation();
  return (
    <div className="flex gap-3 py-5" role="status" aria-label={t("chat.typing")}>
      <OraMark size="sm" />
      <div className="flex items-center gap-1 py-2.5">
        <span className="size-1.5 animate-pulse rounded-full bg-muted-foreground" style={{ animationDelay: "0ms" }} />
        <span className="size-1.5 animate-pulse rounded-full bg-muted-foreground" style={{ animationDelay: "160ms" }} />
        <span className="size-1.5 animate-pulse rounded-full bg-muted-foreground" style={{ animationDelay: "320ms" }} />
      </div>
    </div>
  );
}
