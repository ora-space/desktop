import { create } from "zustand";

interface UnreadSessionsState {
  /** Ora session ids that finished a turn while the user was looking elsewhere. */
  unread: Set<string>;
  /** Flags a session as having an unread update; a no-op if it already carries one. */
  markUnread: (sessionId: string) => void;
  /** Clears the unread flag for a session; a no-op if it had none. */
  markRead: (sessionId: string) => void;
}

/**
 * Purely client-side "unread" ledger for agent sessions.
 *
 * The backend has no concept of read state, so this never leaves the browser: a
 * session is marked unread when a turn ends while it is not the active selection
 * and cleared the moment the user opens it. Kept out of the chat store because
 * "have I looked at this yet" is a shell concern, not conversation content.
 */
export const useUnreadSessionsStore = create<UnreadSessionsState>((set) => ({
  unread: new Set<string>(),
  markUnread: (sessionId) =>
    set((state) =>
      state.unread.has(sessionId)
        ? state
        : { unread: new Set(state.unread).add(sessionId) },
    ),
  markRead: (sessionId) =>
    set((state) => {
      if (!state.unread.has(sessionId)) return state;
      const next = new Set(state.unread);
      next.delete(sessionId);
      return { unread: next };
    }),
}));
