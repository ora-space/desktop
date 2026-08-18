import { createContext, useContext } from "react";
import type { SessionArtifactIndex } from "./artifact-index";

export interface ChatLinkContextValue {
  index: SessionArtifactIndex;
  taskId: string;
  cwd?: string | null;
}

export const ChatLinkContext = createContext<ChatLinkContextValue | null>(null);

/** Returns the session artifact index when the thread is inside a task conversation. */
export function useChatLinkContext(): ChatLinkContextValue | null {
  return useContext(ChatLinkContext);
}
