import type { acp } from "@ora/contracts";
import { createStore, type StoreApi } from "zustand/vanilla";
import type { AcpClient } from "./client.js";

/** Identifies who produced a rendered chat message. */
export type ChatMessageRole = "user" | "assistant";

/** Represents one fully assembled message in an Ora session conversation. */
export interface ChatMessage {
  id: string;
  role: ChatMessageRole;
  content: string;
  createdAt: number;
}

/** Holds the in-memory chat state isolated to one stable Ora session identifier. */
export interface SessionConversation {
  messages: ChatMessage[];
  isResponding: boolean;
  error: string | null;
}

/** Supplies the identities needed to route one user prompt and its streamed reply. */
export interface SendMessageRequest {
  oraSessionId: string;
  agentSessionId: string;
  text: string;
}

/** Exposes chat state and protocol-backed actions from one isolated store instance. */
export interface ChatState {
  conversations: Record<string, SessionConversation>;
  newSession(request: acp.NewSessionRequest): Promise<acp.NewSessionResponse>;
  sendMessage(request: SendMessageRequest): Promise<void>;
  clearAll(): void;
  dispose(): void;
}

/** Dependencies that make message identity and timestamps deterministic in tests. */
export interface ChatStoreOptions {
  createId?: () => string;
  now?: () => number;
}

export type ChatStore = StoreApi<ChatState>;

const EMPTY_CONVERSATION: SessionConversation = {
  messages: [],
  isResponding: false,
  error: null,
};

/** Creates one in-memory chat store and binds it to the supplied ACP client. */
export function createChatStore(
  client: AcpClient,
  options: ChatStoreOptions = {},
): ChatStore {
  const createId = options.createId ?? (() => crypto.randomUUID());
  const now = options.now ?? Date.now;
  const oraSessionByAgentSession = new Map<string, string>();
  let unsubscribe = (): void => undefined;

  const store = createStore<ChatState>((set, get) => ({
    conversations: {},

    newSession: (request) => client.newSession(request),

    sendMessage: async ({ oraSessionId, agentSessionId, text }) => {
      const content = text.trim();
      if (content === "") return;

      const conversation = get().conversations[oraSessionId] ?? EMPTY_CONVERSATION;
      if (conversation.isResponding) {
        throw new Error("this Ora session is already processing a prompt");
      }

      oraSessionByAgentSession.set(agentSessionId, oraSessionId);
      const userMessage: ChatMessage = {
        id: createId(),
        role: "user",
        content,
        createdAt: now(),
      };
      updateConversation(set, oraSessionId, (current) => ({
        ...current,
        messages: [...current.messages, userMessage],
        isResponding: true,
        error: null,
      }));

      try {
        await client.prompt({
          sessionId: agentSessionId,
          prompt: [{ type: "text", text: content }],
        });
      } catch (error) {
        updateConversation(set, oraSessionId, (current) => ({
          ...current,
          error: errorMessage(error),
        }));
        throw error;
      } finally {
        updateConversation(set, oraSessionId, (current) => ({
          ...current,
          isResponding: false,
        }));
      }
    },

    clearAll: () => set({ conversations: {} }),

    dispose: () => {
      unsubscribe();
      oraSessionByAgentSession.clear();
    },
  }));

  unsubscribe = client.subscribe((notification) => {
    const oraSessionId = oraSessionByAgentSession.get(notification.sessionId);
    if (oraSessionId === undefined) return;

    const update = notification.update;
    if (update.sessionUpdate !== "agent_message_chunk") return;
    if (update.messageId === undefined || update.messageId === null) {
      updateConversation(store.setState, oraSessionId, (current) => ({
        ...current,
        error: "ACP agent message chunk is missing messageId",
      }));
      return;
    }
    if (update.content.type !== "text") return;

    appendAgentChunk(
      store.setState,
      oraSessionId,
      update.messageId,
      update.content.text,
      now(),
    );
  });

  return store;
}

/** Appends a text chunk to its ACP message, creating the message on the first chunk. */
function appendAgentChunk(
  set: ChatStore["setState"],
  oraSessionId: string,
  messageId: string,
  text: string,
  createdAt: number,
): void {
  updateConversation(set, oraSessionId, (conversation) => {
    const messageIndex = conversation.messages.findIndex(
      (message) => message.id === messageId,
    );
    if (messageIndex === -1) {
      return {
        ...conversation,
        messages: [
          ...conversation.messages,
          { id: messageId, role: "assistant", content: text, createdAt },
        ],
      };
    }

    const messages = [...conversation.messages];
    const message = messages[messageIndex]!;
    messages[messageIndex] = { ...message, content: message.content + text };
    return { ...conversation, messages };
  });
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
      [oraSessionId]: update(
        state.conversations[oraSessionId] ?? EMPTY_CONVERSATION,
      ),
    },
  }));
}

/** Produces a stable user-facing message for unknown promise rejection values. */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "ACP request failed";
}
