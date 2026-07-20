import { useEffect, useRef } from "react";
import { OraMark } from "../../components/ora-mark";
import { useTranslation } from "react-i18next";
import { MessageBubble } from "./message-bubble";
import type { ChatMessage } from "@ora/chat";

interface MessageListProps {
  messages: ChatMessage[];
  userName: string;
  isResponding: boolean;
}

/** The scrollable message thread, kept pinned to the latest message. */
export function MessageList({ messages, userName, isResponding }: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastMessage = messages.at(-1);
  const showTyping = isResponding && lastMessage?.role !== "assistant";

  // Keep the latest message in view as the thread grows or the assistant "types".
  // Streaming appends fire on every chunk, so those scroll instantly; only whole
  // new messages animate, otherwise the smooth scroll never settles mid-stream.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.style.scrollBehavior = isResponding ? "auto" : "smooth";
    el.scrollTop = el.scrollHeight;
  }, [messages.length, lastMessage?.content, isResponding]);

  return (
    <div
      ref={scrollRef}
      data-testid="message-list"
      aria-live="polite"
      className="scrollbar-hide flex-1 overflow-y-auto"
    >
      <div className="mx-auto w-full max-w-[760px] px-3 pb-4 pt-5 sm:px-5 sm:pt-8">
        {messages.map((message) => (
          <MessageBubble key={message.id} message={message} userName={userName} />
        ))}
        {showTyping && <TypingIndicator />}
        <div className="h-8" />
      </div>
    </div>
  );
}

/** Three bouncing dots shown while the assistant prepares a reply. */
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
