import { useEffect, useRef } from "react";
import { OraMark } from "../../components/ora-mark";
import { MessageBubble } from "./message-bubble";
import type { ChatMessage } from "../../lib/types";

interface MessageListProps {
  messages: ChatMessage[];
  userName: string;
  isResponding: boolean;
}

/** The scrollable message thread, kept pinned to the latest message. */
export function MessageList({ messages, userName, isResponding }: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Keep the latest message in view as the thread grows or the assistant "types".
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, isResponding]);

  return (
    <div ref={scrollRef} className="scrollbar-hide flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-3xl px-4">
        {messages.map((message) => (
          <MessageBubble key={message.id} message={message} userName={userName} />
        ))}
        {isResponding && <TypingIndicator />}
        <div className="h-4" />
      </div>
    </div>
  );
}

/** Three bouncing dots shown while the assistant prepares a reply. */
function TypingIndicator() {
  return (
    <div className="flex gap-3 py-4" aria-label="Assistant is typing">
      <OraMark size="sm" />
      <div className="flex items-center gap-1 py-2.5">
        <span className="size-2 animate-bounce rounded-full bg-fg-quaternary" style={{ animationDelay: "0ms" }} />
        <span className="size-2 animate-bounce rounded-full bg-fg-quaternary" style={{ animationDelay: "150ms" }} />
        <span className="size-2 animate-bounce rounded-full bg-fg-quaternary" style={{ animationDelay: "300ms" }} />
      </div>
    </div>
  );
}
