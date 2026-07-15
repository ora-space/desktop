import { useState } from "react";
import type { ContractsClient } from "@ora/contracts";
import { ContractsClientContext } from "./contracts-client-context";
import { ChatView } from "./features/chat/chat-view";
import { Sidebar } from "./features/sidebar/sidebar";
import { CONVERSATIONS_STORAGE_KEY, useConversations } from "./hooks/use-conversations";
import { CURRENT_USER } from "./lib/mock-data";
import type { CurrentUser } from "./lib/types";

interface AppShellProps {
  client: ContractsClient;
  user?: CurrentUser;
}

/** The main Ora application shell: sidebar + chat view with conversation state. */
export function AppShell({ client, user = CURRENT_USER }: AppShellProps) {
  const {
    conversations,
    activeId,
    activeConversation,
    isResponding,
    newChat,
    selectConversation,
    sendMessage,
    renameConversation,
    removeConversation,
  } = useConversations();

  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  const handleSignOut = () => {
    try {
      window.localStorage.removeItem(CONVERSATIONS_STORAGE_KEY);
    } catch {
      /* Storage may be unavailable; reload anyway. */
    }
    window.location.reload();
  };

  return (
    <ContractsClientContext.Provider value={client}>
      <div className="flex h-dvh bg-primary text-primary">
        <Sidebar
          user={user}
          conversations={conversations}
          activeId={activeId}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={() => setSidebarCollapsed((collapsed) => !collapsed)}
          onNewChat={newChat}
          onSelectConversation={selectConversation}
          onRenameConversation={renameConversation}
          onRemoveConversation={removeConversation}
          onSignOut={handleSignOut}
        />
        <ChatView
          active={activeConversation}
          userName={user.name}
          isResponding={isResponding}
          onSend={sendMessage}
          onNewChat={newChat}
        />
      </div>
    </ContractsClientContext.Provider>
  );
}
