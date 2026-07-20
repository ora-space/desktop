import { createContext, useContext } from "react";
import type { ChatStore } from "@ora/chat";

export const ChatStoreContext = createContext<ChatStore | null>(null);

/** Returns the chat store explicitly bound to the current application shell. */
export function useChatStore(): ChatStore {
  const store = useContext(ChatStoreContext);
  if (store === null) {
    throw new Error("useChatStore must be used within AppShell");
  }
  return store;
}
