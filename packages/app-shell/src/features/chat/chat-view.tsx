import { Edit01 } from "@untitledui/icons";
import { IconButton } from "../../components/icon-button";
import { Composer } from "./composer";
import { EmptyState } from "./empty-state";
import { MessageList } from "./message-list";
import type { Conversation } from "../../lib/types";

interface ChatViewProps {
  active: Conversation | null;
  userName: string;
  isResponding: boolean;
  onSend: (text: string) => void;
  onNewChat: () => void;
}

/** The right pane: a centered empty composer, or a thread + composer. */
export function ChatView({ active, userName, isResponding, onSend, onNewChat }: ChatViewProps) {
  if (!active) {
    return (
      <main className="flex flex-1 flex-col bg-primary">
        <EmptyState onSend={onSend} />
      </main>
    );
  }

  return (
    <main className="flex flex-1 flex-col bg-primary">
      <header className="flex h-14 shrink-0 items-center gap-2 border-b border-secondary px-3">
        <span className="truncate text-sm font-semibold text-primary">{active.title}</span>
        <div className="flex-1" />
        <IconButton icon={Edit01} label="New chat" onClick={onNewChat} />
      </header>

      <MessageList messages={active.messages} userName={userName} isResponding={isResponding} />

      <div className="shrink-0 px-4 pb-4">
        <div className="mx-auto w-full max-w-3xl">
          <Composer onSend={onSend} isResponding={isResponding} />
        </div>
      </div>
    </main>
  );
}
