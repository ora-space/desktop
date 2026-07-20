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

      <div className="shrink-0 px-4 pb-4">
        <div className="mx-auto w-full max-w-3xl">
          {error && <p role="alert" className="mb-2 text-xs text-destructive">{error}</p>}
          <Composer onSend={onSend} isResponding={isResponding} disabled={disabled} />
        </div>
      </div>
    </main>
  );
}
