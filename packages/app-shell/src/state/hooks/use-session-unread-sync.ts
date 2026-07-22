import { useEffect } from "react";
import type { ChatStore } from "@ora/chat";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useUnreadSessionsStore } from "../stores/unread-sessions-store";

/**
 * Maintains the client-side unread ledger from live prompt activity.
 *
 * A session earns an unread mark when one of its turns ends (`isResponding`
 * flips true -> false) while it is not the active selection - the case where the
 * agent finished something the user was not watching. Opening the session clears
 * it. Both signals already live in the shell (the chat store and the selection
 * store), so this stays frontend-only and never touches the backend.
 */
export function useSessionUnreadSync(chatStore: ChatStore): void {
  useEffect(() => {
    const { markUnread, markRead } = useUnreadSessionsStore.getState();

    // Remember each session's last-seen responding state so we react only to the
    // true -> false edge, not to every streamed chunk in between. Seed from the
    // current store so a turn that ended before mount is not flagged in arrears.
    const wasResponding = new Map<string, boolean>();
    for (const [id, conversation] of Object.entries(chatStore.getState().conversations)) {
      wasResponding.set(id, conversation.isResponding);
    }

    const unsubscribeChat = chatStore.subscribe((state) => {
      const selectedId = useWorkspaceSelectionStore.getState().selection.sessionId;
      for (const [id, conversation] of Object.entries(state.conversations)) {
        const responding = conversation.isResponding;
        const finishedTurn = wasResponding.get(id) === true && !responding;
        wasResponding.set(id, responding);
        // A turn that ends while its session is open is already read.
        if (finishedTurn && id !== selectedId) markUnread(id);
      }
    });

    const unsubscribeSelection = useWorkspaceSelectionStore.subscribe((state) => {
      if (state.selection.sessionId !== null) markRead(state.selection.sessionId);
    });

    return () => {
      unsubscribeChat();
      unsubscribeSelection();
    };
  }, [chatStore]);
}
