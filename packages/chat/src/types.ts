import type * as acp from "@agentclientprotocol/sdk";
import type { RunDiagnostics } from "./diagnostics.ts";
import type {
  SessionHistoryNotice,
  SessionPermissionRequest,
  TokenUsageReport,
} from "@ora/contracts";

/** Identifies who produced a rendered chat message. */
export type ChatMessageRole = "user" | "assistant";

/** Preserves explicit provider attribution without inferring a child session lifecycle. */
export interface ChatItemMetadata {
  contentContext?: { metadata?: unknown; annotations?: unknown };
  metadata?: Record<string, unknown>;
}

/** Represents one fully assembled text message in an Ora session conversation. */
export interface ChatMessage extends ChatItemMetadata {
  kind: "message";
  id: string;
  role: ChatMessageRole;
  content: string;
  structuredContent?: Array<Exclude<acp.ContentBlock, { type: "text" }>>;
  createdAt: number;
  protocolMessageId?: string;
}

/** Represents streamed agent progress that is visually secondary to the final answer. */
export interface ChatThought extends ChatItemMetadata {
  kind: "thought";
  id: string;
  content: string;
  createdAt: number;
  protocolMessageId?: string;
}

/** Stores the latest complete plan snapshot for one response turn. */
export interface ChatPlan extends ChatItemMetadata {
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
export type ChatToolCallStatus = acp.ToolCallStatus | "cancelled" | "unknown";

/** Stores one tool call and its latest ACP lifecycle fields. */
export interface ChatToolCall extends ChatItemMetadata {
  kind: "toolCall";
  id: string;
  title: string;
  toolKind?: acp.ToolKind;
  status?: ChatToolCallStatus;
  /** True when `status: "completed"` was implied at `end_turn` rather than reported by the provider. */
  settledByTurnEnd?: boolean;
  content: acp.ToolCallContent[];
  locations: acp.ToolCallLocation[];
  rawInput?: unknown;
  rawOutput?: unknown;

  createdAt: number;
  startedAt?: number;
  durationMs?: number;
  updatedAt: number;
}

/** Preserves one structured non-text ACP block at its original timeline position. */
export interface ChatContent extends ChatItemMetadata {
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

/**
 * Records that the backend re-sent this turn's prompt after the agent stalled.
 *
 * Counted from the first retry so it reads as `retry / maxRetries`. Live state
 * only: a reloaded transcript carries no retry marker, so the field is absent
 * there and the notice belongs to the streaming turn alone.
 */
export interface ChatTurnRetry {
  retry: number;
  maxRetries: number;
  /** The last retry also stalled and the turn failed with the agent's timeout. */
  exhausted?: boolean;
}

/** Groups one user message with every agent update produced in response. */
export interface ChatTurn {
  id: string;
  userMessage: ChatMessage;
  items: ChatTurnItem[];
  status: ChatTurnStatus;
  stopReason: acp.StopReason | null;
  error: string | null;
  createdAt: number;
  /** Starts after any pre-prompt session preparation, so response time excludes handoff. */
  responseStartedAt?: number;
  durationMs?: number;
  retry?: ChatTurnRetry;
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

/** One non-persisted context snapshot reported by the active agent. */
export interface ContextUsageSnapshot {
  usedTokens: number;
  sizeTokens: number;
  cost?: acp.Cost;
  receivedAt: number;
}

/** Models whether Ora can currently show an agent-authored context snapshot. */
export type ContextUsageState =
  | { status: "hidden" }
  | { status: "needs_interaction" }
  | { status: "awaiting_report" }
  | { status: "unavailable" }
  | { status: "reported"; snapshot: ContextUsageSnapshot };

/** Models the most recently completed prompt's optional draft token report. */
export type LastTurnTokenState =
  | { status: "none" }
  | { status: "awaiting_completion" }
  | { status: "unavailable" }
  | { status: "reported"; usage: TokenUsageReport; receivedAt: number };

/** Owns all volatile usage data for one loaded conversation. */
export interface SessionUsage {
  context: ContextUsageState;
  lastTurnTokens: LastTurnTokenState;
}

/** Holds the in-memory chat state isolated to one stable Ora session identifier. */
export interface SessionConversation {
  /**
   * Live observations only. A prompt Ora streams starts an empty snapshot, so an
   * absent value means no live attempt was observed here (replayed history or
   * none yet) instead of "the attempt happened and showed nothing". Loading
   * history drops the snapshot rather than inventing recent activity.
   */
  diagnostics?: RunDiagnostics;
  /**
   * The agent's configuration selectors (model, and anything else it offers) with
   * their current values. Session-scoped rather than turn-scoped: they arrive
   * when the session is created or loaded, are refreshed by
   * `config_option_update`, and are the only source for what the model picker
   * can show once a session exists.
   */
  configOptions: acp.SessionConfigOption[];
  /**
   * Whether the session is known to have no provider now.
   *
   * Its last options are kept for what they showed, but there is nothing to configure: a choice is
   * an intent for the next attach. Cleared by any options a stream or a read brings.
   */
  providerLost: boolean;
  /** Model switches recorded in this thread, oldest first. */
  modelChanges: ChatModelChange[];
  /** Known holes in Ora's durable record reported by the latest successful replay. */
  historyNotices: SessionHistoryNotice[];
  turns: ChatTurn[];
  /**
   * The slash commands the Agent last reported, or `null` while it has not answered.
   *
   * Opening a conversation never contacts its Agent, so "not asked yet" and "offers none" are
   * different facts; only the second may be presented as a command being unsupported.
   */
  availableCommands: acp.AvailableCommand[] | null;
  sessionTitle: string | null;
  sessionUpdatedAt: string | null;
  isLoaded: boolean;
  isLoading: boolean;
  isResponding: boolean;
  pendingPermissions: SessionPermissionRequest[];
  /** Volatile agent telemetry; intentionally absent from Ora's session history. */
  usage: SessionUsage;
  error: string | null;
}
