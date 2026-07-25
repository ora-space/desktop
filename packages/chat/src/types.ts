import type { acp, SessionPermissionRequest } from "@ora/contracts";

/** Identifies who produced a rendered chat message. */
export type ChatMessageRole = "user" | "assistant";

/** Represents one fully assembled text message in an Ora session conversation. */
export interface ChatMessage {
  kind: "message";
  id: string;
  role: ChatMessageRole;
  content: string;
  structuredContent?: Array<Exclude<acp.ContentBlock, { type: "text" }>>;
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

/** Adds a client-owned terminal state for prompts cancelled before the provider settles a tool. */
export type ChatToolCallStatus = acp.ToolCallStatus | "cancelled";

/** Stores one tool call and its latest ACP lifecycle fields. */
export interface ChatToolCall {
  kind: "toolCall";
  id: string;
  title: string;
  toolKind?: acp.ToolKind;
  status?: ChatToolCallStatus;
  content: acp.ToolCallContent[];
  locations: acp.ToolCallLocation[];
  rawInput?: unknown;
  rawOutput?: unknown;
  createdAt: number;
  updatedAt: number;
}

/** Preserves one structured non-text ACP block at its original timeline position. */
export interface ChatContent {
  kind: "content";
  id: string;
  source: "message" | "thought";
  content: Exclude<acp.ContentBlock, { type: "text" }>;
  createdAt: number;
}

/** One ordered item emitted by the agent during a response turn. */
export type ChatTurnItem =
  | ChatMessage
  | ChatThought
  | ChatPlan
  | ChatToolCall
  | ChatContent;

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
  availableCommands: acp.AvailableCommand[];
  sessionTitle: string | null;
  sessionUpdatedAt: string | null;
  isLoaded: boolean;
  isLoading: boolean;
  isResponding: boolean;
  pendingPermissions: SessionPermissionRequest[];
  error: string | null;
}
