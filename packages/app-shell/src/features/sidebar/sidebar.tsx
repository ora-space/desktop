import { useMemo, useState } from "react";
import { Edit01, Menu04, SearchMd } from "@untitledui/icons";
import { Input } from "@ora/ui";
import { IconButton } from "../../components/icon-button";
import { ConversationItem } from "./conversation-item";
import { UserProfile } from "./user-profile";
import { groupConversationsByDate } from "../../lib/grouping";
import type { Conversation, CurrentUser } from "../../lib/types";

interface SidebarProps {
  user: CurrentUser;
  conversations: Conversation[];
  activeId: string | null;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  onNewChat: () => void;
  onSelectConversation: (id: string) => void;
  onRenameConversation: (id: string, title: string) => void;
  onRemoveConversation: (id: string) => void;
  onSignOut: () => void;
}

/** The collapsible left rail: new chat, search, date-grouped history, user footer. */
export function Sidebar({
  user,
  conversations,
  activeId,
  collapsed,
  onToggleCollapsed,
  onNewChat,
  onSelectConversation,
  onRenameConversation,
  onRemoveConversation,
  onSignOut,
}: SidebarProps) {
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return conversations;
    return conversations.filter((c) => c.title.toLowerCase().includes(needle));
  }, [conversations, query]);

  const groups = useMemo(() => groupConversationsByDate(filtered, Date.now()), [filtered]);

  if (collapsed) {
    return (
      <aside className="flex h-dvh w-16 shrink-0 flex-col items-center gap-2 border-r border-secondary bg-secondary py-2">
        <IconButton icon={Edit01} label="New chat" onClick={onNewChat} />
        <IconButton icon={Menu04} label="Open sidebar" onClick={onToggleCollapsed} />
        <div className="flex-1" />
        <UserProfile user={user} compact onSignOut={onSignOut} />
      </aside>
    );
  }

  return (
    <aside className="flex h-dvh w-72 shrink-0 flex-col border-r border-secondary bg-secondary">
      <div className="flex items-center justify-between px-2 pt-2">
        <IconButton icon={Edit01} label="New chat" onClick={onNewChat} />
        <IconButton icon={Menu04} label="Collapse sidebar" onClick={onToggleCollapsed} />
      </div>

      <div className="px-2 pt-2">
        <Input size="sm" icon={SearchMd} placeholder="Search conversations" value={query} onChange={setQuery} />
      </div>

      <nav className="scrollbar-hide mt-1 flex-1 overflow-y-auto px-2 py-1">
        {groups.length === 0 ? (
          <p className="px-2 py-6 text-center text-sm text-tertiary">No conversations found.</p>
        ) : (
          groups.map((group) => (
            <section key={group.label} className="mb-1">
              <p className="px-2 py-1 text-xs font-semibold text-quaternary">{group.label}</p>
              <div className="space-y-px">
                {group.conversations.map((conversation) => (
                  <ConversationItem
                    key={conversation.id}
                    conversation={conversation}
                    active={conversation.id === activeId}
                    onSelect={() => onSelectConversation(conversation.id)}
                    onRename={(title) => onRenameConversation(conversation.id, title)}
                    onRemove={() => onRemoveConversation(conversation.id)}
                  />
                ))}
              </div>
            </section>
          ))
        )}
      </nav>

      <div className="border-t border-secondary p-2">
        <UserProfile user={user} onSignOut={onSignOut} />
      </div>
    </aside>
  );
}
