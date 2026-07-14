/** Roles a chat message can originate from. */
export type Role = "user" | "assistant";

/** A single message inside a conversation. */
export interface ChatMessage {
  id: string;
  role: Role;
  content: string;
  /** Epoch milliseconds at which the message was created. */
  createdAt: number;
}

/** A conversation thread, ordered oldest-to-newest by `messages[].createdAt`. */
export interface Conversation {
  id: string;
  title: string;
  messages: ChatMessage[];
  createdAt: number;
  /** Last activity, used for sidebar ordering and date grouping. */
  updatedAt: number;
}

/** The signed-in user surfaced in the sidebar footer. */
export interface CurrentUser {
  name: string;
  email: string;
}
