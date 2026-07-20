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
  // Whether the thread should keep chasing the newest content. Reading back a
  // scroll position during a stream is unreliable, so intent is tracked here
  // instead of being recomputed from the DOM at append time.
  const followTailRef = useRef(true);
  const lastMessage = messages.at(-1);
  const showTyping = isResponding && lastMessage?.role !== "assistant";

  // Scrolling away mid-stream is how the user says "stop chasing the tail"; coming
  // back within a line of the bottom re-arms it. The threshold absorbs fractional
  // scroll heights, which otherwise leave the thread permanently unpinned.
  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    followTailRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };

  // Sending always re-pins: the user just produced the newest message, so they
  // are asking to see it even if they had scrolled up to read history.
  useEffect(() => {
    if (lastMessage?.role === "user") followTailRef.current = true;
  }, [messages.length, lastMessage?.role]);

  // Keep the latest message in view as the thread grows or the assistant "types".
  // Streaming appends fire on every chunk, so those scroll instantly; only whole
  // new messages animate, otherwise the smooth scroll never settles mid-stream.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !followTailRef.current) return;
    el.style.scrollBehavior = isResponding ? "auto" : "smooth";
    el.scrollTop = el.scrollHeight;
  }, [messages.length, lastMessage?.content, isResponding]);

  // `min-h-0` lets the thread shrink below its content height; without it the
  // list wins the space fight and shoves the composer past the window bottom.
  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      data-testid="message-list"
      aria-live="polite"
      className="scrollbar-hide min-h-0 flex-1 animate-in overflow-y-auto fade-in duration-500"
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
