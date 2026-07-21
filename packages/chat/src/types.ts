import type { acp } from "@ora/contracts";

/** Identifies who produced a rendered chat message. */
export type ChatMessageRole = "user" | "assistant";

/** Represents one fully assembled text message in an Ora session conversation. */
export interface ChatMessage {
  kind: "message";
  id: string;
  role: ChatMessageRole;
  content: string;
  createdAt: number;
  protocolMessageId?: string;
}

/** Represents streamed agent progress that is visually secondary to the final answer. */
export interface ChatThought {
  kind: "thought";
  id: string;
  content: string;
  createdAt: number;
  protocolMessageId?: string;
}

/** Stores the latest complete plan snapshot for one response turn. */
export interface ChatPlan {
  kind: "plan";
  id: string;
  entries: acp.PlanEntry[];
  createdAt: number;
  updatedAt: number;
}

/** Stores one tool call and its latest ACP lifecycle fields. */
export interface ChatToolCall {
  kind: "toolCall";
  id: string;
  title: string;
  toolKind?: acp.ToolKind;
  status?: acp.ToolCallStatus;
  content: acp.ToolCallContent[];
  locations: acp.ToolCallLocation[];
  rawInput?: unknown;
  rawOutput?: unknown;
  createdAt: number;
  updatedAt: number;
}

/** Keeps unsupported ACP content visible without forcing the renderer to understand it. */
export interface ChatUnsupportedContent {
  kind: "unsupportedContent";
  id: string;
  source: "message" | "thought";
  contentType: Exclude<acp.ContentBlock["type"], "text">;
  createdAt: number;
}

/** One ordered item emitted by the agent during a response turn. */
export type ChatTurnItem =
  | ChatMessage
  | ChatThought
  | ChatPlan
  | ChatToolCall
  | ChatUnsupportedContent;

/** Describes the lifecycle of one user prompt and its agent response. */
export type ChatTurnStatus = "streaming" | "completed" | "cancelled" | "failed";

/** Groups one user message with every agent update produced in response. */
export interface ChatTurn {
  id: string;
  userMessage: ChatMessage;
  items: ChatTurnItem[];
  status: ChatTurnStatus;
  stopReason: acp.StopReason | null;
  error: string | null;
  createdAt: number;
}

/** Holds the in-memory chat state isolated to one stable Ora session identifier. */
export interface SessionConversation {
  turns: ChatTurn[];
  error: string | null;
}

/** Returns whether the conversation currently owns an active response turn. */
export function isConversationResponding(conversation: SessionConversation | undefined): boolean {
  return conversation?.turns.at(-1)?.status === "streaming";
}
