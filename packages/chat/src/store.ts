import type {
  acp,
  ContractsClient,
  SessionPermissionRequest,
} from "@ora/contracts";
import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  ChatPlan,
  ChatToolCall,
  ChatTurn,
  SessionConversation,
} from "./types.js";

export type {
  ChatMessage,
  ChatMessageRole,
  ChatPlan,
  ChatThought,
  ChatToolCall,
  ChatTurn,
  ChatTurnItem,
  ChatTurnStatus,
  ChatUnsupportedContent,
  SessionConversation,
} from "./types.js";

export interface SendMessageRequest {
  text: string;
  /**
   * Target an existing session. Provide this OR `createSession`, never both:
   * with an id the prompt streams straight into that session, without one the
   * session is created lazily (see `createSession`).
   */
  oraSessionId?: string;
  /**
   * Lazily creates the backing agent session, resolving to its real id. When
   * present, the user turn is materialized immediately under a temporary key and
   * the (slow, cold-start) session creation runs in the background, so the
   * composer slides into the thread without waiting on the agent handshake. The
   * store re-keys the conversation onto the real id once it arrives.
   */
  createSession?: () => Promise<string>;
  /**
   * Fired synchronously once the optimistic turn exists, carrying the temporary
   * key so the caller can select the draft conversation for display. Only
   * relevant on the `createSession` path.
   */
  onDraft?: (draftSessionId: string) => void;
  /** Fired after the store has re-keyed onto the real session id. */
  onSessionCreated?: (oraSessionId: string) => void;
}

export interface ChatState {
  conversations: Record<string, SessionConversation>;
  loadSession(oraSessionId: string): Promise<void>;
  sendMessage(request: SendMessageRequest): Promise<void>;
  stopGeneration(oraSessionId: string): void;
  respondToPermission(oraSessionId: string, permissionRequestId: string, optionId: string): Promise<void>;
  clearAll(): void;
  dispose(): void;
}

export interface ChatStoreOptions {
  createId?: () => string;
  now?: () => number;
}

export type ChatStore = StoreApi<ChatState>;
export type ChatSessionClient = Pick<
  ContractsClient["session"],
  "load" | "prompt" | "respondToPermission"
>;

const EMPTY_CONVERSATION: SessionConversation = {
  turns: [],
  isLoaded: false,
  isLoading: false,
  isResponding: false,
  pendingPermissions: [],
  error: null,
};

/** Creates a per-session chat state owner backed directly by generated Ora contracts. */
export function createChatStore(
  client: ChatSessionClient,
  options: ChatStoreOptions = {},
): ChatStore {
  const createId = options.createId ?? (() => crypto.randomUUID());
  const now = options.now ?? Date.now;
  const operations = new Map<string, AbortController>();

  const store = createStore<ChatState>((set, get) => ({
    conversations: {},

    loadSession: async (oraSessionId) => {
      if (operations.has(oraSessionId)) return;
      const previous = get().conversations[oraSessionId] ?? EMPTY_CONVERSATION;
      const controller = new AbortController();
      const staged = new HistoryBuilder(createId, now);
      let completed = false;
      operations.set(oraSessionId, controller);
      updateConversation(set, oraSessionId, () => ({
        ...previous,
        turns: [],
        isLoading: true,
        error: null,
      }));
      try {
        for await (const event of client.load(
          { sessionId: oraSessionId },
          { signal: controller.signal },
        )) {
          if (event.type === "session_update") {
            staged.applyUpdate(event.update);
          } else if (event.type === "permission_request") {
            staged.addPermission(event);
          } else {
            completed = true;
          }
        }
        if (!completed) {
          throw new Error("agent session load ended before completion");
        }
        updateConversation(set, oraSessionId, () => ({
          ...EMPTY_CONVERSATION,
          turns: staged.finish(),
          pendingPermissions: staged.permissions,
          isLoaded: true,
          isLoading: false,
        }));
      } catch (error) {
        updateConversation(set, oraSessionId, () => ({
          ...previous,
          error: isAbortError(error) ? previous.error : errorMessage(error),
        }));
        if (!isAbortError(error)) throw error;
      } finally {
        operations.delete(oraSessionId);
        updateConversation(set, oraSessionId, (conversation) => ({
          ...conversation,
          isLoading: false,
        }));
      }
    },

    sendMessage: async ({ oraSessionId, text, createSession, onDraft, onSessionCreated }) => {
      const content = text.trim();
      if (content === "") return;

      // Stream into the given session, or a temporary draft key that is promoted
      // to the real id once the background create resolves. The key is mutable
      // because every store write below has to follow it across that promotion.
      let key = oraSessionId ?? `draft-${createId()}`;
      if (operations.has(key)) {
        throw new Error("this Ora session is already processing an operation");
      }
      const controller = new AbortController();
      operations.set(key, controller);

      const createdAt = now();
      const turnId = createId();
      const turn: ChatTurn = {
        id: turnId,
        userMessage: {
          kind: "message",
          id: createId(),
          role: "user",
          content,
          createdAt,
        },
        items: [],
        status: "streaming",
        stopReason: null,
        error: null,
        createdAt,
      };
      updateConversation(set, key, (conversation) => ({
        ...conversation,
        turns: [...conversation.turns, turn],
        isResponding: true,
        error: null,
      }));
      // Let the caller show the draft conversation before we block on the agent.
      onDraft?.(key);

      if (createSession) {
        let realId: string;
        try {
          realId = await createSession();
        } catch (error) {
          // Nothing streamed yet; settle the draft turn and stop here.
          const message = errorMessage(error);
          updateTurn(set, key, turnId, (current) => ({ ...current, status: "failed", error: message }));
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            isResponding: false,
            error: message,
          }));
          operations.delete(key);
          throw error;
        }
        if (controller.signal.aborted) {
          // Stopped mid-startup: the session exists but we never open its stream.
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming" ? { ...current, status: "cancelled" } : current,
          );
          updateConversation(set, key, (conversation) => ({ ...conversation, isResponding: false }));
          operations.delete(key);
          return;
        }
        // Carry the live conversation and its operation onto the real id, then let
        // the caller re-point selection at it. Order matters: the conversation is
        // re-keyed before selection moves so the new id never reads as empty.
        promoteConversation(set, key, realId);
        operations.delete(key);
        operations.set(realId, controller);
        key = realId;
        // This turn was streamed live, so the local conversation already is the
        // session's history. Marking it loaded stops the workspace's "load if not
        // loaded" effect from firing once the session id resolves — that reload
        // clears turns to empty first, which would bounce the composer back to the
        // landing layout and replay the slide-down animation.
        updateConversation(set, key, (conversation) => ({ ...conversation, isLoaded: true }));
        onSessionCreated?.(realId);
      }

      try {
        for await (const event of client.prompt(
          { sessionId: key, text: content },
          { signal: controller.signal },
        )) {
          if (event.type === "session_update") {
            // The user turn is already materialized, so the echoed prompt chunk
            // would only duplicate it; every other update belongs to this turn.
            if (event.update.sessionUpdate === "user_message_chunk") continue;
            updateTurn(set, key, turnId, (current) =>
              applyAgentUpdate(current, event.update, createId, now()),
            );
          } else if (event.type === "permission_request") {
            appendPermission(set, key, event);
          } else {
            updateTurn(set, key, turnId, (current) => ({
              ...current,
              status: event.stopReason === "cancelled" ? "cancelled" : "completed",
              stopReason: event.stopReason,
            }));
          }
        }
      } catch (error) {
        if (isAbortError(error)) {
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming" ? { ...current, status: "cancelled" } : current,
          );
          clearPendingPermissions(set, key);
        } else {
          const message = errorMessage(error);
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming"
              ? { ...current, status: "failed", error: message }
              : current,
          );
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            error: message,
          }));
          throw error;
        }
      } finally {
        operations.delete(key);
        updateTurn(set, key, turnId, (current) =>
          current.status === "streaming" ? { ...current, status: "completed" } : current,
        );
        updateConversation(set, key, (conversation) => ({
          ...conversation,
          isResponding: false,
        }));
      }
    },

    stopGeneration: (oraSessionId) => operations.get(oraSessionId)?.abort(),

    respondToPermission: async (oraSessionId, permissionRequestId, optionId) => {
      try {
        await client.respondToPermission({
          sessionId: oraSessionId,
          permissionRequestId,
          optionId,
        });
        updateConversation(set, oraSessionId, (conversation) => ({
          ...conversation,
          pendingPermissions: conversation.pendingPermissions.filter(
            (request) => request.permissionRequestId !== permissionRequestId,
          ),
          error: null,
        }));
      } catch (error) {
        updateConversation(set, oraSessionId, (conversation) => ({
          ...conversation,
          error: errorMessage(error),
        }));
        throw error;
      }
    },

    clearAll: () => set({ conversations: {} }),
    dispose: () => {
      operations.forEach((controller) => controller.abort());
      operations.clear();
    },
  }));

  return store;
}

/**
 * Reconstructs turn boundaries from a replayed provider history, where a user
 * message chunk starts a new turn and every other update flows into it.
 */
class HistoryBuilder {
  readonly permissions: SessionPermissionRequest[] = [];
  private readonly turns: ChatTurn[] = [];

  constructor(
    private readonly createId: () => string,
    private readonly now: () => number,
  ) {}

  applyUpdate(update: acp.SessionUpdate): void {
    if (update.sessionUpdate === "user_message_chunk") {
      this.appendUserChunk(update);
      return;
    }
    const turn = this.currentTurn();
    this.replaceLast(applyAgentUpdate(turn, update, this.createId, this.now()));
  }

  addPermission(request: SessionPermissionRequest): void {
    this.permissions.push(request);
  }

  /** Marks every replayed turn as finished, since a stopped session has no live work. */
  finish(): ChatTurn[] {
    return this.turns.map((turn) => ({ ...turn, status: "completed" as const }));
  }

  private appendUserChunk(chunk: acp.ContentChunk): void {
    const last = this.turns.at(-1);
    const protocolMessageId = chunk.messageId ?? undefined;
    const continuesUser =
      last !== undefined &&
      last.items.length === 0 &&
      last.userMessage.role === "user" &&
      (protocolMessageId === undefined || last.userMessage.protocolMessageId === protocolMessageId);
    if (chunk.content.type === "text" && continuesUser && last) {
      this.replaceLast({
        ...last,
        userMessage: { ...last.userMessage, content: last.userMessage.content + chunk.content.text },
      });
      return;
    }
    const createdAt = this.now();
    this.turns.push({
      id: this.createId(),
      userMessage: {
        kind: "message",
        id: this.createId(),
        role: "user",
        content: chunk.content.type === "text" ? chunk.content.text : "",
        createdAt,
        ...(protocolMessageId === undefined ? {} : { protocolMessageId }),
      },
      items: [],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt,
    });
  }

  /** Ensures an agent update always has a turn, even before any user message replays. */
  private currentTurn(): ChatTurn {
    const last = this.turns.at(-1);
    if (last !== undefined) return last;
    const createdAt = this.now();
    const turn: ChatTurn = {
      id: this.createId(),
      userMessage: { kind: "message", id: this.createId(), role: "user", content: "", createdAt },
      items: [],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt,
    };
    this.turns.push(turn);
    return turn;
  }

  private replaceLast(turn: ChatTurn): void {
    this.turns[this.turns.length - 1] = turn;
  }
}

/** Normalizes one agent-produced ACP update into a response turn's ordered items. */
function applyAgentUpdate(
  turn: ChatTurn,
  update: acp.SessionUpdate,
  createId: () => string,
  timestamp: number,
): ChatTurn {
  switch (update.sessionUpdate) {
    case "agent_message_chunk":
      return appendContentChunk(turn, "message", update, createId, timestamp);
    case "agent_thought_chunk":
      return appendContentChunk(turn, "thought", update, createId, timestamp);
    case "plan":
      return replacePlan(turn, update.entries, timestamp);
    case "tool_call":
      return upsertToolCall(turn, update, timestamp);
    case "tool_call_update":
      return updateToolCall(turn, update, timestamp);
    default:
      return turn;
  }
}

/** Aggregates text chunks and records a visible placeholder for unsupported content. */
function appendContentChunk(
  turn: ChatTurn,
  itemKind: "message" | "thought",
  chunk: acp.ContentChunk,
  createId: () => string,
  timestamp: number,
): ChatTurn {
  const content = chunk.content;
  if (content.type !== "text") {
    return {
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
    };
  }

  const protocolMessageId = chunk.messageId ?? undefined;
  const implicitId = `${itemKind}-implicit-${turn.id}`;
  const itemId = protocolMessageId === undefined ? implicitId : `${itemKind}-${protocolMessageId}`;
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
}

/** Replaces the current turn's complete plan snapshot without changing its timeline position. */
function replacePlan(turn: ChatTurn, entries: acp.PlanEntry[], timestamp: number): ChatTurn {
  const planIndex = turn.items.findIndex((item) => item.kind === "plan");
  if (planIndex === -1) {
    const plan: ChatPlan = {
      kind: "plan",
      id: `plan-${turn.id}`,
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
}

/** Inserts a new tool call or replaces its complete initial snapshot. */
function upsertToolCall(turn: ChatTurn, toolCall: acp.ToolCall, timestamp: number): ChatTurn {
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
}

/** Applies the partial fields from one ACP tool update to its existing timeline item. */
function updateToolCall(turn: ChatTurn, update: acp.ToolCallUpdate, timestamp: number): ChatTurn {
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
}

function appendPermission(
  set: ChatStore["setState"],
  oraSessionId: string,
  request: SessionPermissionRequest,
): void {
  updateConversation(set, oraSessionId, (conversation) => ({
    ...conversation,
    pendingPermissions: [...conversation.pendingPermissions, request],
  }));
}

/** Clears requests that the backend settles as cancelled with the aborted prompt. */
function clearPendingPermissions(set: ChatStore["setState"], oraSessionId: string): void {
  updateConversation(set, oraSessionId, (conversation) => ({
    ...conversation,
    pendingPermissions: [],
  }));
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

/** Moves a conversation from a temporary draft key onto its real session id. */
function promoteConversation(
  set: ChatStore["setState"],
  fromKey: string,
  toKey: string,
): void {
  set((state) => {
    const conversation = state.conversations[fromKey];
    if (conversation === undefined) return state;
    const conversations = { ...state.conversations, [toKey]: conversation };
    delete conversations[fromKey];
    return { conversations };
  });
}

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

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Agent request failed";
}
