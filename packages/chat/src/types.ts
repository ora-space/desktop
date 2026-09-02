import type * as acp from "@agentclientprotocol/sdk";
import type {
  SessionHistoryNotice,
  SessionPermissionRequest,
} from "@ora/contracts";

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

/**
 * Adds a client-owned terminal state for tools the provider never settled.
 *
 * ACP has no status for work whose outcome was never reported, and a turn that
 * was cancelled, cut short, or broken by a failed stream leaves exactly that.
 * The state is distinct from `failed` on purpose: the tool is not known to have
 * failed, only to have been interrupted before anything said otherwise.
 */
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
  ChatMessage | ChatThought | ChatPlan | ChatToolCall | ChatContent;

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

/**
 * Marks the point in a thread where the answering model changed.
 *
 * Kept beside the turns rather than inside them because a switch happens between
 * turns — often while the thread is idle — and belongs to no prompt or response.
 */
export interface ChatModelChange {
  id: string;
  /** How many turns preceded the switch, which is where it renders in the thread. */
  afterTurnCount: number;
  /** The human-readable name of the model that took over. */
  modelName: string;
  createdAt: number;
}

/** Holds the in-memory chat state isolated to one stable Ora session identifier. */
export interface SessionConversation {
  /**
   * The agent's configuration selectors (model, and anything else it offers) with
   * their current values. Session-scoped rather than turn-scoped: they arrive
   * when the session is created or loaded, are refreshed by
   * `config_option_update`, and are the only source for what the model picker
   * can show once a session exists.
   */
  configOptions: acp.SessionConfigOption[];
  /** Model switches recorded in this thread, oldest first. */
  modelChanges: ChatModelChange[];
  /** Known holes in Ora's durable record reported by the latest successful replay. */
  historyNotices: SessionHistoryNotice[];
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
