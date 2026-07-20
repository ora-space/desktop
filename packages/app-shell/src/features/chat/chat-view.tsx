import { Composer } from "./composer";
import { EmptyState } from "./empty-state";
import { MessageList } from "./message-list";
import type { ChatMessage } from "@ora/chat";

interface ChatViewProps {
  messages: ChatMessage[];
  userName: string;
  isResponding: boolean;
  error: string | null;
  disabled?: boolean;
  onSend: (text: string) => void;
}

/** The right pane: a centered empty composer, or a thread + composer. */
export function ChatView({ messages, userName, isResponding, error, disabled = false, onSend }: ChatViewProps) {
  if (messages.length === 0) {
    return (
      <main className="flex flex-1 flex-col bg-background">
        <EmptyState onSend={onSend} isResponding={isResponding} error={error} disabled={disabled} />
      </main>
    );
  }

  return (
    <main className="flex flex-1 flex-col bg-background">
      <MessageList messages={messages} userName={userName} isResponding={isResponding} />

      {/* Gradient fade so the thread dissolves under the composer instead of hard-clipping. */}
      <div className="shrink-0 bg-gradient-to-t from-background via-background to-transparent px-3 pb-4 pt-6 sm:px-5">
        <div className="mx-auto w-full max-w-[760px]">
          {error && <p role="alert" className="mb-2 px-1 text-xs text-destructive">{error}</p>}
          <Composer onSend={onSend} isResponding={isResponding} disabled={disabled} />
        </div>
      </div>
    </main>
  );
}
