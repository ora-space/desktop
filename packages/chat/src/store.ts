import type { acp } from "@ora/contracts";
import { createStore, type StoreApi } from "zustand/vanilla";
import type { AcpClient } from "./client.js";
import type {
  ChatMessage,
  ChatPlan,
  ChatToolCall,
  ChatTurn,
  SessionConversation,
} from "./types.js";

/** Supplies the identities needed to route one user prompt and its streamed reply. */
export interface SendMessageRequest {
  oraSessionId: string;
  agentSessionId: string;
  text: string;
}

/** Identifies the active turn that should be cancelled. */
export interface CancelMessageRequest {
  oraSessionId: string;
  agentSessionId: string;
}

/** Exposes chat state and protocol-backed actions from one isolated store instance. */
export interface ChatState {
  conversations: Record<string, SessionConversation>;
  newSession(request: acp.NewSessionRequest): Promise<acp.NewSessionResponse>;
  sendMessage(request: SendMessageRequest): Promise<void>;
  cancelMessage(request: CancelMessageRequest): Promise<void>;
  clearAll(): void;
  dispose(): void;
}

/** Dependencies that make message identity and timestamps deterministic in tests. */
export interface ChatStoreOptions {
  createId?: () => string;
  now?: () => number;
}

export type ChatStore = StoreApi<ChatState>;

interface ActiveTurnRoute {
  oraSessionId: string;
  turnId: string;
}

const EMPTY_CONVERSATION: SessionConversation = {
  turns: [],
  error: null,
};

/** Creates one in-memory chat store and binds it to the supplied ACP client. */
export function createChatStore(
  client: AcpClient,
  options: ChatStoreOptions = {},
): ChatStore {
  const createId = options.createId ?? (() => crypto.randomUUID());
  const now = options.now ?? Date.now;
  const activeTurnByAgentSession = new Map<string, ActiveTurnRoute>();
  let unsubscribe = (): void => undefined;

  const store = createStore<ChatState>((set, get) => ({
    conversations: {},

    newSession: (request) => client.newSession(request),

    sendMessage: async ({ oraSessionId, agentSessionId, text }) => {
      const content = text.trim();
      if (content === "") return;

      const conversation = get().conversations[oraSessionId] ?? EMPTY_CONVERSATION;
      if (conversation.turns.at(-1)?.status === "streaming") {
        throw new Error("this Ora session is already processing a prompt");
      }

      const createdAt = now();
      const turnId = createId();
      const userMessage: ChatMessage = {
        kind: "message",
        id: createId(),
        role: "user",
        content,
        createdAt,
      };
      const turn: ChatTurn = {
        id: turnId,
        userMessage,
        items: [],
        status: "streaming",
        stopReason: null,
        error: null,
        createdAt,
      };

      activeTurnByAgentSession.set(agentSessionId, { oraSessionId, turnId });
      updateConversation(set, oraSessionId, (current) => ({
        turns: [...current.turns, turn],
        error: null,
      }));

      try {
        const response = await client.prompt({
          sessionId: agentSessionId,
          prompt: [{ type: "text", text: content }],
        });
        updateTurn(set, oraSessionId, turnId, (current) => ({
          ...current,
          status: response.stopReason === "cancelled" ? "cancelled" : "completed",
          stopReason: response.stopReason,
        }));
      } catch (error) {
        const message = errorMessage(error);
        updateConversation(set, oraSessionId, (current) => ({
          ...current,
          error: current.turns.find((candidate) => candidate.id === turnId)?.status === "cancelled"
            ? null
            : message,
        }));
        updateTurn(set, oraSessionId, turnId, (current) =>
          current.status === "cancelled"
            ? current
            : { ...current, status: "failed", error: message },
        );
        if (getTurn(get().conversations[oraSessionId], turnId)?.status !== "cancelled") {
          throw error;
        }
      } finally {
        const activeRoute = activeTurnByAgentSession.get(agentSessionId);
        if (activeRoute?.turnId === turnId) activeTurnByAgentSession.delete(agentSessionId);
      }
    },

    cancelMessage: async ({ oraSessionId, agentSessionId }) => {
      const route = activeTurnByAgentSession.get(agentSessionId);
      if (route?.oraSessionId !== oraSessionId) return;
      await client.cancel({ sessionId: agentSessionId });
    },

    clearAll: () => {
      activeTurnByAgentSession.clear();
      set({ conversations: {} });
    },

    dispose: () => {
      unsubscribe();
      activeTurnByAgentSession.clear();
    },
  }));

  unsubscribe = client.subscribe((notification) => {
    const route = activeTurnByAgentSession.get(notification.sessionId);
    if (route === undefined) return;
    const turn = getTurn(store.getState().conversations[route.oraSessionId], route.turnId);
    if (turn?.status !== "streaming") return;

    applySessionUpdate(
      store.setState,
      route.oraSessionId,
      route.turnId,
      notification.update,
      createId,
      now(),
    );
  });

  return store;
}

/** Normalizes one ACP update into the active response turn. */
function applySessionUpdate(
  set: ChatStore["setState"],
  oraSessionId: string,
  turnId: string,
  update: acp.SessionUpdate,
  createId: () => string,
  timestamp: number,
): void {
  switch (update.sessionUpdate) {
    case "agent_message_chunk":
      appendContentChunk(set, oraSessionId, turnId, "message", update, createId, timestamp);
      return;
    case "agent_thought_chunk":
      appendContentChunk(set, oraSessionId, turnId, "thought", update, createId, timestamp);
      return;
    case "plan":
      replacePlan(set, oraSessionId, turnId, update.entries, timestamp);
      return;
    case "tool_call":
      upsertToolCall(set, oraSessionId, turnId, update, timestamp);
      return;
    case "tool_call_update":
      updateToolCall(set, oraSessionId, turnId, update, timestamp);
      return;
    case "user_message_chunk":
    case "available_commands_update":
    case "current_mode_update":
    case "config_option_update":
    case "session_info_update":
    case "usage_update":
      return;
  }
}

/** Aggregates text chunks and records a visible placeholder for unsupported content. */
function appendContentChunk(
  set: ChatStore["setState"],
  oraSessionId: string,
  turnId: string,
  itemKind: "message" | "thought",
  chunk: acp.ContentChunk,
  createId: () => string,
  timestamp: number,
): void {
  const content = chunk.content;
  if (content.type !== "text") {
    updateTurn(set, oraSessionId, turnId, (turn) => ({
      ...turn,
      items: [
        ...turn.items,
        {
          kind: "unsupportedContent",
          id: createId(),
          source: itemKind,
          contentType: content.type as Exclude<acp.ContentBlock["type"], "text">,
          createdAt: timestamp,
        },
      ],
    }));
    return;
  }

  const protocolMessageId = chunk.messageId ?? undefined;
  const implicitId = `${itemKind}-implicit-${turnId}`;
  const itemId = protocolMessageId === undefined ? implicitId : `${itemKind}-${protocolMessageId}`;
  updateTurn(set, oraSessionId, turnId, (turn) => {
    const itemIndex = turn.items.findIndex((item) => item.id === itemId && item.kind === itemKind);
    if (itemIndex === -1) {
      const item = itemKind === "message"
        ? {
          kind: "message" as const,
          id: itemId,
          role: "assistant" as const,
          content: content.text,
          createdAt: timestamp,
          ...(protocolMessageId === undefined ? {} : { protocolMessageId }),
        }
        : {
          kind: "thought" as const,
          id: itemId,
          content: content.text,
          createdAt: timestamp,
          ...(protocolMessageId === undefined ? {} : { protocolMessageId }),
        };
      return { ...turn, items: [...turn.items, item] };
    }

    const items = [...turn.items];
    const item = items[itemIndex]!;
    if (item.kind === "message" || item.kind === "thought") {
      items[itemIndex] = { ...item, content: item.content + content.text };
    }
    return { ...turn, items };
  });
}

/** Replaces the current turn's complete plan snapshot without changing its timeline position. */
function replacePlan(
  set: ChatStore["setState"],
  oraSessionId: string,
  turnId: string,
  entries: acp.PlanEntry[],
  timestamp: number,
): void {
  updateTurn(set, oraSessionId, turnId, (turn) => {
    const planIndex = turn.items.findIndex((item) => item.kind === "plan");
    if (planIndex === -1) {
      const plan: ChatPlan = {
        kind: "plan",
        id: `plan-${turnId}`,
        entries,
        createdAt: timestamp,
        updatedAt: timestamp,
      };
      return { ...turn, items: [...turn.items, plan] };
    }

    const items = [...turn.items];
    const plan = items[planIndex] as ChatPlan;
    items[planIndex] = { ...plan, entries, updatedAt: timestamp };
    return { ...turn, items };
  });
}

/** Inserts a new tool call or replaces its complete initial snapshot. */
function upsertToolCall(
  set: ChatStore["setState"],
  oraSessionId: string,
  turnId: string,
  toolCall: acp.ToolCall,
  timestamp: number,
): void {
  updateTurn(set, oraSessionId, turnId, (turn) => {
    const toolIndex = turn.items.findIndex(
      (item) => item.kind === "toolCall" && item.id === toolCall.toolCallId,
    );
    const next: ChatToolCall = {
      kind: "toolCall",
      id: toolCall.toolCallId,
      title: toolCall.title,
      ...(toolCall.kind === undefined ? {} : { toolKind: toolCall.kind }),
      ...(toolCall.status === undefined ? {} : { status: toolCall.status }),
      content: toolCall.content ?? [],
      locations: toolCall.locations ?? [],
      ...(toolCall.rawInput === undefined ? {} : { rawInput: toolCall.rawInput }),
      ...(toolCall.rawOutput === undefined ? {} : { rawOutput: toolCall.rawOutput }),
      createdAt: toolIndex === -1 ? timestamp : (turn.items[toolIndex] as ChatToolCall).createdAt,
      updatedAt: timestamp,
    };
    if (toolIndex === -1) return { ...turn, items: [...turn.items, next] };

    const items = [...turn.items];
    items[toolIndex] = next;
    return { ...turn, items };
  });
}

/** Applies the partial fields from one ACP tool update to its existing timeline item. */
function updateToolCall(
  set: ChatStore["setState"],
  oraSessionId: string,
  turnId: string,
  update: acp.ToolCallUpdate,
  timestamp: number,
): void {
  updateTurn(set, oraSessionId, turnId, (turn) => {
    const toolIndex = turn.items.findIndex(
      (item) => item.kind === "toolCall" && item.id === update.toolCallId,
    );
    if (toolIndex === -1) {
      const tool: ChatToolCall = {
        kind: "toolCall",
        id: update.toolCallId,
        title: update.title ?? "Tool call",
        ...(update.kind === undefined || update.kind === null ? {} : { toolKind: update.kind }),
        ...(update.status === undefined || update.status === null ? {} : { status: update.status }),
        content: update.content ?? [],
        locations: update.locations ?? [],
        ...(update.rawInput === undefined ? {} : { rawInput: update.rawInput }),
        ...(update.rawOutput === undefined ? {} : { rawOutput: update.rawOutput }),
        createdAt: timestamp,
        updatedAt: timestamp,
      };
      return { ...turn, items: [...turn.items, tool] };
    }

    const items = [...turn.items];
    const current = items[toolIndex] as ChatToolCall;
    items[toolIndex] = {
      ...current,
      ...(update.title === undefined || update.title === null ? {} : { title: update.title }),
      ...(update.kind === undefined ? {} : { toolKind: update.kind ?? undefined }),
      ...(update.status === undefined ? {} : { status: update.status ?? undefined }),
      ...(update.content === undefined ? {} : { content: update.content ?? [] }),
      ...(update.locations === undefined ? {} : { locations: update.locations ?? [] }),
      ...(update.rawInput === undefined ? {} : { rawInput: update.rawInput }),
      ...(update.rawOutput === undefined ? {} : { rawOutput: update.rawOutput }),
      updatedAt: timestamp,
    };
    return { ...turn, items };
  });
}

/** Applies an immutable update to one response turn. */
function updateTurn(
  set: ChatStore["setState"],
  oraSessionId: string,
  turnId: string,
  update: (turn: ChatTurn) => ChatTurn,
): void {
  updateConversation(set, oraSessionId, (conversation) => ({
    ...conversation,
    turns: conversation.turns.map((turn) => (turn.id === turnId ? update(turn) : turn)),
  }));
}

/** Applies an immutable update to one Ora session without affecting concurrent sessions. */
function updateConversation(
  set: ChatStore["setState"],
  oraSessionId: string,
  update: (conversation: SessionConversation) => SessionConversation,
): void {
  set((state) => ({
    conversations: {
      ...state.conversations,
      [oraSessionId]: update(state.conversations[oraSessionId] ?? EMPTY_CONVERSATION),
    },
  }));
}

/** Finds one response turn without exposing mutable store internals. */
function getTurn(conversation: SessionConversation | undefined, turnId: string): ChatTurn | undefined {
  return conversation?.turns.find((turn) => turn.id === turnId);
}

/** Produces a stable user-facing message for unknown promise rejection values. */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "ACP request failed";
}
