import type * as acp from "@agentclientprotocol/sdk";
import {
  type ContractsClient,
  type PromptSessionEvent,
  LocalTransportError,
  RemoteContractError,
  type SessionHistoryNotice,
  type SessionPermissionRequest,
} from "@ora/contracts";
import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  ChatModelChange,
  ChatPlan,
  ChatToolCall,
  ChatToolCallStatus,
  ChatTurn,
  SessionConversation,
} from "./types.js";
import { currentModel } from "./model-option.js";

export type {
  ChatContent,
  ChatMessage,
  ChatMessageRole,
  ChatModelChange,
  ChatPlan,
  ChatThought,
  ChatToolCall,
  ChatToolCallStatus,
  ChatTurn,
  ChatTurnItem,
  ChatTurnStatus,
  SessionConversation,
} from "./types.js";

export interface SendMessageRequest {
  text: string;
  images?: acp.ImageContent[];
  /**
   * What is actually sent to the agent, when it must differ from the displayed
   * `text`. Used to prepend an invisible instruction (e.g. a spec-driven workflow
   * reminder) that the user should not see echoed in their own message. Defaults
   * to `text` when omitted.
   */
  agentText?: string;
  /**
   * The session to stream into. Always known before sending, because a chat
   * surface warms its session when it opens.
   */
  oraSessionId: string;
  /**
   * Runs after the user's turn is on screen and before the prompt is sent, for
   * work the session needs first — creating its Task and persisting it.
   *
   * Keeping it here rather than in the caller is what preserves the immediate
   * composer slide: the turn renders under its final id straight away, and this
   * runs in the gap before the agent is asked anything.
   */
  prepare?: () => Promise<PreparedSession>;
}

/** What persisting a warm session revealed about the agent behind it. */
export interface PreparedSession {
  availableCommands: acp.AvailableCommand[];
}

export interface ChatState {
  conversations: Record<string, SessionConversation>;
  /** Registers a newly created provider session as an empty, already-loaded conversation. */
  initializeSession(oraSessionId: string): void;
  /**
   * Records the configuration selectors an agent reported for one session.
   *
   * Used to seed a warm session's options before any turn exists, from a warm
   * or attach response the store did not issue itself.
   */
  setConfigOptions(
    oraSessionId: string,
    configOptions: acp.SessionConfigOption[],
  ): void;
  /**
   * Adopts the options of an agent this session has just been rebound onto.
   *
   * Separate from `setConfigOptions` only in where it marks the transcript. A
   * move is performed by the message that carries it, so that message is already
   * in the thread and is the first the incoming agent answers — the mark belongs
   * before it rather than after the whole exchange.
   */
  adoptSwitchedAgent(
    oraSessionId: string,
    configOptions: acp.SessionConfigOption[],
  ): void;
  /**
   * Applies one configuration selection — in practice the model — to a session.
   *
   * The agent's reply is authoritative rather than the requested value: an agent
   * that adjusted or rejected the choice describes the result, and that is what
   * gets recorded. Works on a warm session as well as a persisted one, so a model
   * can be chosen before the first message is sent.
   */
  setSessionConfig(
    oraSessionId: string,
    configId: string,
    value: string,
  ): Promise<void>;
  loadSession(oraSessionId: string): Promise<void>;
  sendMessage(request: SendMessageRequest): Promise<void>;
  stopGeneration(oraSessionId: string): void;
  respondToPermission(
    oraSessionId: string,
    permissionRequestId: string,
    optionId: string,
  ): Promise<void>;
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
  "load" | "prompt" | "respondToPermission" | "setConfig"
>;

const EMPTY_CONVERSATION: SessionConversation = {
  configOptions: [],
  modelChanges: [],
  historyNotices: [],
  turns: [],
  availableCommands: [],
  sessionTitle: null,
  sessionUpdatedAt: null,
  isLoaded: false,
  isLoading: false,
  isResponding: false,
  pendingPermissions: [],
  error: null,
};

/** Caps one live text batch so streamed output stays responsive under very large responses. */
const STREAMING_TEXT_BATCH_LIMIT = 4 * 1024;

interface BufferedTextChunk {
  itemKind: "message" | "thought";
  messageId: string | undefined;
  text: string;
  timestamp: number;
}

type ConversationUpdate = Extract<
  acp.SessionUpdate,
  { sessionUpdate: "available_commands_update" | "session_info_update" }
>;

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

    initializeSession: (oraSessionId) => {
      updateConversation(set, oraSessionId, (conversation) => ({
        ...conversation,
        isLoaded: true,
        isLoading: false,
        error: null,
      }));
    },

    setConfigOptions: (oraSessionId, configOptions) => {
      updateConversation(set, oraSessionId, (conversation) =>
        withConfigOptions(
          conversation,
          configOptions,
          createId,
          now(),
          conversation.turns.length,
        ),
      );
    },

    adoptSwitchedAgent: (oraSessionId, configOptions) => {
      updateConversation(set, oraSessionId, (conversation) =>
        withConfigOptions(
          conversation,
          configOptions,
          createId,
          now(),
          // The turn that carries the move is already on screen and is the first
          // one the incoming agent answers, so the mark belongs before it rather
          // than after the exchange it introduced.
          Math.max(conversation.turns.length - 1, 0),
        ),
      );
    },

    setSessionConfig: async (oraSessionId, configId, value) => {
      try {
        const { configOptions } = await client.setConfig({
          sessionId: oraSessionId,
          configId,
          value,
        });
        updateConversation(set, oraSessionId, (conversation) => ({
          ...withConfigOptions(
            conversation,
            configOptions,
            createId,
            now(),
            conversation.turns.length,
          ),
          error: null,
        }));
      } catch (error) {
        // Only the round trip itself can fail here — a rejected selection comes
        // back as a successful response carrying the agent's own options. So the
        // picker must not silently snap back to its old value; report it like
        // every other session action that could not reach the agent.
        updateConversation(set, oraSessionId, (conversation) => ({
          ...conversation,
          error: errorMessage(error),
        }));
        throw error;
      }
    },

    loadSession: async (oraSessionId) => {
      if (operations.has(oraSessionId)) return;
      const previous = get().conversations[oraSessionId] ?? EMPTY_CONVERSATION;
      const controller = new AbortController();
      const staged = new HistoryBuilder(createId, now);
      let completed = false;
      let pendingPreviewTimer: ReturnType<typeof setTimeout> | null = null;

      /** Publishes the active replay turn without repainting once per provider text token. */
      const flushPendingPreview = () => {
        if (pendingPreviewTimer !== null) {
          clearTimeout(pendingPreviewTimer);
          pendingPreviewTimer = null;
        }
        const preview = staged.preview(previous.configOptions);
        if (preview.isResponding) {
          updateConversation(set, oraSessionId, () => preview);
          return;
        }
        // Historical turns look open between their first chunk and turn_ended.
        // Always clear a previously published live flag when the turn settles;
        // otherwise the sidebar keeps the working icon until the whole load ends.
        updateConversation(set, oraSessionId, (conversation) =>
          conversation.isResponding
            ? { ...conversation, isResponding: false }
            : conversation,
        );
      };

      /** Matches live prompt rendering by limiting text-only replay updates to one per frame. */
      const schedulePendingPreview = () => {
        if (pendingPreviewTimer !== null) return;
        pendingPreviewTimer = setTimeout(() => {
          pendingPreviewTimer = null;
          flushPendingPreview();
        }, 16);
      };

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
          let batchPreview = false;
          if (event.type === "session_update") {
            // Session-scoped updates are split out before the turn accumulator
            // sees them; they describe the conversation, not any one turn.
            const configOptions = sessionScopedConfigOptions(event.update);
            if (configOptions) {
              staged.configOptions = configOptions;
            } else {
              staged.applyUpdate(event.update);
              batchPreview =
                (event.update.sessionUpdate === "user_message_chunk" ||
                  event.update.sessionUpdate === "agent_message_chunk" ||
                  event.update.sessionUpdate === "agent_thought_chunk") &&
                event.update.content.type === "text";
            }
          } else if (event.type === "permission_request") {
            staged.addPermission(event);
          } else if (event.type === "turn_ended") {
            // Preserve the final text batch while the turn is still live; ending it first would
            // make preview deliberately decline to publish a non-responding intermediate state.
            if (pendingPreviewTimer !== null) flushPendingPreview();
            staged.endTurn(event.stopReason);
          } else if (event.type === "history_notice") {
            staged.addHistoryNotice(event.notice);
          } else {
            completed = true;
          }
          if (!completed) {
            // A workflow-owned prompt may already be active when load starts. Publish only that
            // open turn incrementally; completed-history loads remain staged until completion.
            if (batchPreview) schedulePendingPreview();
            else flushPendingPreview();
          }
        }
        if (!completed) {
          throw new Error("agent session load ended before completion");
        }
        updateConversation(set, oraSessionId, () => ({
          ...staged.finish(),
          // An agent that reports nothing on load leaves whatever the warm
          // session already established, rather than blanking the picker.
          configOptions: staged.configOptions ?? previous.configOptions,
          // Replay rebuilds the transcript from the provider, which knows
          // nothing about Ora's markers, so any earlier ones cannot be placed
          // against the new turns and are dropped rather than misplaced.
          modelChanges: [],
        }));
      } catch (error) {
        updateConversation(set, oraSessionId, () => ({
          ...previous,
          error: isAbortError(error) ? previous.error : errorMessage(error),
        }));
        if (!isAbortError(error)) throw error;
      } finally {
        if (pendingPreviewTimer !== null) {
          clearTimeout(pendingPreviewTimer);
          pendingPreviewTimer = null;
        }
        operations.delete(oraSessionId);
        updateConversation(set, oraSessionId, (conversation) => ({
          ...conversation,
          isLoading: false,
        }));
      }
    },

    sendMessage: async ({
      oraSessionId,
      text,
      images = [],
      agentText,
      prepare,
    }) => {
      const content = text.trim();
      if (content === "" && images.length === 0) return;
      // What the agent receives can differ from what the user sees in their turn,
      // so a workflow reminder is sent without appearing in the transcript.
      const promptContent = (agentText ?? text).trim();
      const prompt: acp.ContentBlock[] = [
        ...(promptContent === ""
          ? []
          : [{ type: "text" as const, text: promptContent }]),
        ...images.map((image) => ({ type: "image" as const, ...image })),
      ];
      // What Ora records as the user turn. When injected material (a workflow
      // reminder or role/skill expansion) differs from the displayed text, the
      // backend records this clean copy so history keeps the user's own chips;
      // otherwise it is omitted and the prompt is recorded verbatim.
      const recordPrompt: acp.ContentBlock[] | undefined =
        promptContent === content
          ? undefined
          : [
              ...(content === ""
                ? []
                : [{ type: "text" as const, text: content }]),
              ...images.map((image) => ({ type: "image" as const, ...image })),
            ];

      const key = oraSessionId;
      if (operations.has(key)) {
        throw new Error("this Ora session is already processing an operation");
      }
      const controller = new AbortController();
      operations.set(key, controller);
      let pendingTextChunk: BufferedTextChunk | null = null;
      let pendingFlushTimer: ReturnType<typeof setTimeout> | null = null;

      /** Flushes one buffered text batch into the live turn before a boundary update. */
      const flushPendingTextChunk = () => {
        if (pendingFlushTimer !== null) {
          clearTimeout(pendingFlushTimer);
          pendingFlushTimer = null;
        }
        const batch = pendingTextChunk;
        if (batch === null) return;
        updateTurn(set, key, turnId, (current) =>
          appendTextContentChunk(
            current,
            batch.itemKind,
            batch.messageId,
            batch.text,
            batch.timestamp,
          ),
        );
        pendingTextChunk = null;
      };

      /** Schedules a near-term flush so streaming remains visible without repainting every token. */
      const schedulePendingTextFlush = () => {
        if (pendingFlushTimer !== null) return;
        pendingFlushTimer = setTimeout(() => {
          pendingFlushTimer = null;
          flushPendingTextChunk();
        }, 16);
      };

      /** Collects one text chunk so repeated provider frames collapse into larger UI updates. */
      const queueTextChunk = (
        itemKind: "message" | "thought",
        chunk: acp.ContentChunk,
      ) => {
        if (chunk.content.type !== "text") return;
        if (
          pendingTextChunk !== null &&
          (pendingTextChunk.itemKind !== itemKind ||
            pendingTextChunk.messageId !== (chunk.messageId ?? undefined))
        ) {
          flushPendingTextChunk();
        }
        if (pendingTextChunk === null) {
          pendingTextChunk = {
            itemKind,
            messageId: chunk.messageId ?? undefined,
            text: chunk.content.text,
            timestamp: now(),
          };
        } else {
          pendingTextChunk.text += chunk.content.text;
        }
        if (pendingTextChunk.text.length >= STREAMING_TEXT_BATCH_LIMIT) {
          flushPendingTextChunk();
        } else {
          schedulePendingTextFlush();
        }
      };

      const createdAt = now();
      const turnId = createId();
      const turn: ChatTurn = {
        id: turnId,
        userMessage: {
          kind: "message",
          id: createId(),
          role: "user",
          content,
          ...(images.length === 0
            ? {}
            : {
                structuredContent: images.map((image) => ({
                  type: "image" as const,
                  ...image,
                })),
              }),
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
      if (prepare) {
        let prepared: PreparedSession;
        try {
          prepared = await prepare();
        } catch (error) {
          // Nothing streamed yet; settle the optimistic turn and stop here.
          const message = errorMessage(error);
          updateTurn(set, key, turnId, (current) => ({
            ...current,
            status: "failed",
            error: message,
          }));
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
            current.status === "streaming"
              ? { ...current, status: "cancelled" }
              : current,
          );
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            isResponding: false,
          }));
          operations.delete(key);
          return;
        }
        // This turn was streamed live, so the local conversation already is the
        // session's history. Marking it loaded stops the workspace's "load if not
        // loaded" effect from firing once the session becomes selectable — that
        // reload clears turns to empty first, which would bounce the composer back
        // to the landing layout and replay the slide-down animation.
        updateConversation(set, key, (conversation) => ({
          ...conversation,
          availableCommands: prepared.availableCommands,
          isLoaded: true,
        }));
      }

      try {
        for await (const event of promptWithReattach(
          client,
          key,
          prompt,
          recordPrompt,
          controller.signal,
        )) {
          if (event.type === "session_update") {
            // The user turn is already materialized, so the echoed prompt chunk
            // would only duplicate it; every other update belongs to this turn.
            const update = event.update;
            if (update.sessionUpdate === "user_message_chunk") continue;
            // An agent may change its own configuration mid-turn; that describes
            // the session, so it never reaches the turn accumulator.
            const configOptions = sessionScopedConfigOptions(update);
            if (configOptions) {
              updateConversation(set, key, (conversation) => ({
                ...conversation,
                configOptions,
              }));
              continue;
            }
            if (isConversationUpdate(update)) {
              updateConversation(set, key, (conversation) =>
                applyConversationUpdate(conversation, update),
              );
              continue;
            }
            if (isDeferredConversationUpdate(update)) continue;
            if (
              update.sessionUpdate === "agent_message_chunk" &&
              update.content.type === "text"
            ) {
              queueTextChunk("message", update);
              continue;
            }
            if (
              update.sessionUpdate === "agent_thought_chunk" &&
              update.content.type === "text"
            ) {
              queueTextChunk("thought", update);
              continue;
            }
            flushPendingTextChunk();
            updateTurn(set, key, turnId, (current) =>
              applyAgentUpdate(current, update, createId, now()),
            );
          } else if (event.type === "permission_request") {
            flushPendingTextChunk();
            appendPermission(set, key, event);
          } else {
            flushPendingTextChunk();
            updateTurn(set, key, turnId, (current) =>
              settleActiveToolCalls(
                {
                  ...current,
                  status:
                    event.stopReason === "cancelled"
                      ? ("cancelled" as const)
                      : ("completed" as const),
                  stopReason: event.stopReason,
                },
                impliedToolStatus(event.stopReason),
                now(),
              ),
            );
          }
        }
      } catch (error) {
        flushPendingTextChunk();
        if (isAbortError(error)) {
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming"
              ? settleActiveToolCalls(
                  { ...current, status: "cancelled" },
                  "cancelled",
                  now(),
                )
              : current,
          );
          clearPendingPermissions(set, key);
        } else {
          const message = errorMessage(error);
          // The failure ended the turn, so tools the agent never settled were
          // interrupted by it. They are not marked failed: the stream broke, and
          // whether the tool itself succeeded is exactly what was never reported.
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming"
              ? settleActiveToolCalls(
                  { ...current, status: "failed", error: message },
                  "cancelled",
                  now(),
                )
              : current,
          );
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            error: message,
          }));
          throw error;
        }
      } finally {
        flushPendingTextChunk();
        operations.delete(key);
        // A stream that ended without a boundary event still closes the turn, so
        // its tools settle with it rather than outliving the turn that owns them.
        // Nothing reported them finishing, so they close as interrupted.
        updateTurn(set, key, turnId, (current) =>
          current.status === "streaming"
            ? settleActiveToolCalls(
                { ...current, status: "completed" },
                "cancelled",
                now(),
              )
            : current,
        );
        updateConversation(set, key, (conversation) => ({
          ...conversation,
          isResponding: false,
        }));
      }
    },

    stopGeneration: (oraSessionId) => operations.get(oraSessionId)?.abort(),

    respondToPermission: async (
      oraSessionId,
      permissionRequestId,
      optionId,
    ) => {
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
 * Reconstructs turn boundaries from Ora's replayed history, where a user message
 * chunk starts a new turn and every other update flows into it.
 *
 * Unlike provider replay, this stream carries explicit turn boundaries, so a turn
 * that was cancelled or failed is restored as such instead of being flattened
 * into a completed one.
 */
class HistoryBuilder {
  readonly permissions: SessionPermissionRequest[] = [];
  /** Durable-record warnings staged until the finite load completes successfully. */
  readonly historyNotices: SessionHistoryNotice[] = [];
  /** Session-scoped options seen during replay; `null` when the agent reported none. */
  configOptions: acp.SessionConfigOption[] | null = null;
  private readonly turns: ChatTurn[] = [];
  private availableCommands: acp.AvailableCommand[] = [];
  private sessionTitle: string | null = null;
  private sessionUpdatedAt: string | null = null;
  /**
   * Whether the last turn is still accepting content.
   *
   * Two prompts in a row that produced no agent output are otherwise
   * indistinguishable from one prompt sent in two chunks; the recorded turn
   * boundary is what tells them apart.
   */
  private hasOpenTurn = false;

  constructor(
    private readonly createId: () => string,
    private readonly now: () => number,
  ) {}

  applyUpdate(update: acp.SessionUpdate): void {
    if (update.sessionUpdate === "user_message_chunk") {
      this.appendUserChunk(update);
      return;
    }
    if (isConversationUpdate(update)) {
      const conversation = applyConversationUpdate(this.snapshot(), update);
      this.availableCommands = conversation.availableCommands;
      this.sessionTitle = conversation.sessionTitle;
      this.sessionUpdatedAt = conversation.sessionUpdatedAt;
      return;
    }
    if (isDeferredConversationUpdate(update)) return;
    const turn = this.currentTurn();
    this.replaceLast(applyAgentUpdate(turn, update, this.createId, this.now()));
  }

  addPermission(request: SessionPermissionRequest): void {
    this.permissions.push(request);
  }

  /** Retains one backend-authored integrity notice without guessing a turn position. */
  addHistoryNotice(notice: SessionHistoryNotice): void {
    this.historyNotices.push(notice);
  }

  /** Settles the open turn with the outcome the record captured for it. */
  endTurn(stopReason: acp.StopReason): void {
    const last = this.turns.at(-1);
    this.hasOpenTurn = false;
    if (last === undefined) return;
    const settled: ChatTurn = {
      ...last,
      status: stopReason === "cancelled" ? "cancelled" : "completed",
      stopReason,
    };
    this.replaceLast(
      settleActiveToolCalls(settled, impliedToolStatus(stopReason), this.now()),
    );
  }

  /** Produces a complete loaded conversation after the finite replay stream ends. */
  finish(): SessionConversation {
    return {
      ...this.snapshot(),
      // A turn still streaming here never reached its boundary in the record,
      // which is what an interrupted process leaves behind. Its tools close with
      // it as interrupted too, so replay neither restores work that appears to
      // still be running nor credits it with an outcome the record never held.
      turns: this.turns.map((turn) =>
        turn.status === "streaming"
          ? settleActiveToolCalls(
              { ...turn, status: "completed" as const },
              "cancelled",
              this.now(),
            )
          : turn,
      ),
      pendingPermissions: this.permissions,
      isLoaded: true,
    };
  }

  /** Materializes safe partial replay state while a load stream is still producing events. */
  preview(
    fallbackConfigOptions: acp.SessionConfigOption[],
  ): SessionConversation {
    return {
      ...this.snapshot(),
      configOptions: this.configOptions ?? fallbackConfigOptions,
      modelChanges: [],
      pendingPermissions: [...this.permissions],
      isLoading: true,
      isResponding: this.hasOpenTurn,
    };
  }

  /** Materializes replay metadata so it can share live-update normalization. */
  private snapshot(): SessionConversation {
    return {
      ...EMPTY_CONVERSATION,
      turns: [...this.turns],
      historyNotices: [...this.historyNotices],
      availableCommands: this.availableCommands,
      sessionTitle: this.sessionTitle,
      sessionUpdatedAt: this.sessionUpdatedAt,
    };
  }

  private appendUserChunk(chunk: acp.ContentChunk): void {
    const last = this.turns.at(-1);
    const protocolMessageId = chunk.messageId ?? undefined;
    const continuesUser =
      this.hasOpenTurn &&
      last !== undefined &&
      last.items.length === 0 &&
      last.userMessage.role === "user" &&
      (protocolMessageId === undefined ||
        last.userMessage.protocolMessageId === protocolMessageId);
    if (continuesUser && last) {
      this.replaceLast({
        ...last,
        userMessage:
          chunk.content.type === "text"
            ? {
                ...last.userMessage,
                content: last.userMessage.content + chunk.content.text,
              }
            : {
                ...last.userMessage,
                structuredContent: [
                  ...(last.userMessage.structuredContent ?? []),
                  chunk.content,
                ],
              },
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
        ...(chunk.content.type === "text"
          ? {}
          : { structuredContent: [chunk.content] }),
        createdAt,
        ...(protocolMessageId === undefined ? {} : { protocolMessageId }),
      },
      items: [],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt,
    });
    this.hasOpenTurn = true;
  }

  /** Ensures an agent update always has a turn, even before any user message replays. */
  private currentTurn(): ChatTurn {
    const last = this.turns.at(-1);
    if (this.hasOpenTurn && last !== undefined) return last;
    const createdAt = this.now();
    const turn: ChatTurn = {
      id: this.createId(),
      userMessage: {
        kind: "message",
        id: this.createId(),
        role: "user",
        content: "",
        createdAt,
      },
      items: [],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt,
    };
    this.turns.push(turn);
    this.hasOpenTurn = true;
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

/** Aggregates text chunks and preserves structured content for dedicated renderers. */
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
          kind: "content",
          id: createId(),
          source: itemKind,
          content,
          createdAt: timestamp,
        },
      ],
    };
  }

  return appendTextContentChunk(
    turn,
    itemKind,
    chunk.messageId ?? undefined,
    content.text,
    timestamp,
  );
}

/** Appends one live text batch while preserving the per-message identity rules. */
/**
 * Finds the text run one chunk continues, or -1 when it starts a new one.
 *
 * A `messageId` is ACP stating which chunks form one message, so it identifies
 * the run outright and the message stays whole however much interleaves with it.
 * Text carrying none has no identity to merge on, and contiguity stands in for
 * one exactly as the recorded history does: an entry that took its own place in
 * the turn — a tool call, a plan, an image — proves later text is a new run.
 * Merging across it would keep appending to an item positioned before that entry
 * and render the agent's summary above the work it describes.
 *
 * The two text streams do not close each other, so reasoning interleaved with an
 * answer still belongs to one message, matching how the record is assembled.
 */
function textRunIndex(
  turn: ChatTurn,
  itemKind: "message" | "thought",
  messageId: string | undefined,
): number {
  if (messageId !== undefined) {
    return turn.items.findIndex(
      (item) => item.kind === itemKind && item.protocolMessageId === messageId,
    );
  }
  for (let index = turn.items.length - 1; index >= 0; index -= 1) {
    const item = turn.items[index]!;
    if (item.kind !== "message" && item.kind !== "thought") return -1;
    if (item.kind === itemKind && item.protocolMessageId === undefined)
      return index;
  }
  return -1;
}

function appendTextContentChunk(
  turn: ChatTurn,
  itemKind: "message" | "thought",
  messageId: string | undefined,
  text: string,
  timestamp: number,
): ChatTurn {
  const itemIndex = textRunIndex(turn, itemKind, messageId);
  if (itemIndex === -1) {
    // An agent switching to a tool call commonly emits empty or newline-only
    // deltas first. Opening a run on those would place a blank block ahead of
    // the work that follows it, so a run starts where its content does.
    if (text.trim() === "") return turn;
    // Implicit runs are numbered by where they begin, which keeps them distinct
    // now that one turn can hold several.
    const itemId =
      messageId === undefined
        ? `${itemKind}-implicit-${turn.id}-${turn.items.length}`
        : `${itemKind}-${messageId}`;
    const item =
      itemKind === "message"
        ? {
            kind: "message" as const,
            id: itemId,
            role: "assistant" as const,
            content: text,
            createdAt: timestamp,
            ...(messageId === undefined
              ? {}
              : { protocolMessageId: messageId }),
          }
        : {
            kind: "thought" as const,
            id: itemId,
            content: text,
            createdAt: timestamp,
            ...(messageId === undefined
              ? {}
              : { protocolMessageId: messageId }),
          };
    return { ...turn, items: [...turn.items, item] };
  }

  const items = [...turn.items];
  const item = items[itemIndex]!;
  if (item.kind === "message" || item.kind === "thought") {
    items[itemIndex] = { ...item, content: item.content + text };
  }
  return { ...turn, items };
}

/** Identifies updates that belong to the conversation chrome rather than a response turn. */
function isConversationUpdate(
  update: acp.SessionUpdate,
): update is ConversationUpdate {
  return (
    update.sessionUpdate === "available_commands_update" ||
    update.sessionUpdate === "session_info_update"
  );
}

/** Ignores deferred conversation chrome without materializing an empty replay turn. */
function isDeferredConversationUpdate(update: acp.SessionUpdate): boolean {
  return (
    update.sessionUpdate === "config_option_update" ||
    update.sessionUpdate === "current_mode_update" ||
    update.sessionUpdate === "usage_update"
  );
}

/** Applies the complete command list or partial session metadata update. */
function applyConversationUpdate(
  conversation: SessionConversation,
  update: ConversationUpdate,
): SessionConversation {
  switch (update.sessionUpdate) {
    case "available_commands_update":
      return { ...conversation, availableCommands: update.availableCommands };
    case "session_info_update":
      return {
        ...conversation,
        sessionTitle:
          update.title === undefined ? conversation.sessionTitle : update.title,
        sessionUpdatedAt:
          update.updatedAt === undefined
            ? conversation.sessionUpdatedAt
            : update.updatedAt,
      };
  }
}

/** Replaces the current turn's complete plan snapshot without changing its timeline position. */
function replacePlan(
  turn: ChatTurn,
  entries: acp.PlanEntry[],
  timestamp: number,
): ChatTurn {
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
function upsertToolCall(
  turn: ChatTurn,
  toolCall: acp.ToolCall,
  timestamp: number,
): ChatTurn {
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
    ...(toolCall.rawOutput === undefined
      ? {}
      : { rawOutput: toolCall.rawOutput }),
    createdAt:
      toolIndex === -1
        ? timestamp
        : (turn.items[toolIndex] as ChatToolCall).createdAt,
    updatedAt: timestamp,
  };
  if (toolIndex === -1) return { ...turn, items: [...turn.items, next] };

  const items = [...turn.items];
  items[toolIndex] = next;
  return { ...turn, items };
}

/** Applies the partial fields from one ACP tool update to its existing timeline item. */
function updateToolCall(
  turn: ChatTurn,
  update: acp.ToolCallUpdate,
  timestamp: number,
): ChatTurn {
  const toolIndex = turn.items.findIndex(
    (item) => item.kind === "toolCall" && item.id === update.toolCallId,
  );
  if (toolIndex === -1) {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: update.toolCallId,
      title: update.title ?? "Tool call",
      ...(update.kind === undefined || update.kind === null
        ? {}
        : { toolKind: update.kind }),
      ...(update.status === undefined || update.status === null
        ? {}
        : { status: update.status }),
      content: update.content ?? [],
      locations: update.locations ?? [],
      ...(update.rawInput === undefined ? {} : { rawInput: update.rawInput }),
      ...(update.rawOutput === undefined
        ? {}
        : { rawOutput: update.rawOutput }),
      createdAt: timestamp,
      updatedAt: timestamp,
    };
    return { ...turn, items: [...turn.items, tool] };
  }

  const items = [...turn.items];
  const current = items[toolIndex] as ChatToolCall;
  items[toolIndex] = {
    ...current,
    ...(update.title === undefined || update.title === null
      ? {}
      : { title: update.title }),
    ...(update.kind === undefined
      ? {}
      : { toolKind: update.kind ?? undefined }),
    ...(update.status === undefined
      ? {}
      : { status: update.status ?? undefined }),
    ...(update.content === undefined ? {} : { content: update.content ?? [] }),
    ...(update.locations === undefined
      ? {}
      : { locations: update.locations ?? [] }),
    ...(update.rawInput === undefined ? {} : { rawInput: update.rawInput }),
    ...(update.rawOutput === undefined ? {} : { rawOutput: update.rawOutput }),
    updatedAt: timestamp,
  };
  return { ...turn, items };
}

/**
 * Settles tools whose provider lifecycle never reached a terminal status.
 *
 * ACP does not require an agent to report a tool call finishing, and one that
 * simply moves on to its next update leaves the call active for the life of the
 * conversation. A turn that ended is proof it is not running, so the outcome the
 * turn implies replaces a status the conversation has already left behind.
 */
function settleActiveToolCalls(
  turn: ChatTurn,
  status: ChatToolCallStatus,
  timestamp: number,
): ChatTurn {
  return {
    ...turn,
    items: turn.items.map((item) =>
      item.kind === "toolCall" &&
      (item.status === "pending" || item.status === "in_progress")
        ? { ...item, status, updatedAt: timestamp }
        : item,
    ),
  };
}

/**
 * Names the tool outcome a turn boundary implies for work the agent never settled.
 *
 * Only a turn the agent ended on its own terms is evidence that the call it left
 * open ran to completion. Every other ending — cancelled, out of tokens, refused —
 * cut the turn short, so the call is shown as interrupted rather than credited
 * with a success nobody reported.
 */
function impliedToolStatus(stopReason: acp.StopReason): ChatToolCallStatus {
  return stopReason === "end_turn" ? "completed" : "cancelled";
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
function clearPendingPermissions(
  set: ChatStore["setState"],
  oraSessionId: string,
): void {
  updateConversation(set, oraSessionId, (conversation) => ({
    ...conversation,
    pendingPermissions: [],
  }));
}

/**
 * Adopts one reported option set, marking the transcript if it switched models.
 *
 * `afterTurnCount` is where the mark belongs, which is not always the end of the
 * thread: options reported for the turn currently being sent describe a change
 * that takes effect *before* it, so the caller decides rather than this.
 */
function withConfigOptions(
  conversation: SessionConversation,
  configOptions: acp.SessionConfigOption[],
  createId: () => string,
  timestamp: number,
  afterTurnCount: number,
): SessionConversation {
  return {
    ...conversation,
    configOptions,
    modelChanges: recordModelChange(
      conversation,
      configOptions,
      createId,
      timestamp,
      afterTurnCount,
    ),
  };
}

/**
 * Records a model switch in the transcript when one actually happened.
 *
 * Deliberately silent in three cases. There is nothing to divide before the
 * first turn, so choosing a model on an empty chat — or moving agent before
 * anything has been exchanged — leaves no mark. The first options a session
 * reports establish the baseline rather than change it. And repeated switches at
 * the same point in the thread collapse into one line, so cycling through the
 * menu does not stack up dividers.
 */
function recordModelChange(
  conversation: SessionConversation,
  configOptions: acp.SessionConfigOption[],
  createId: () => string,
  timestamp: number,
  afterTurnCount: number,
): ChatModelChange[] {
  const previous = currentModel(conversation.configOptions);
  const next = currentModel(configOptions);
  if (
    previous === null ||
    next === null ||
    previous.value === next.value ||
    afterTurnCount === 0
  ) {
    return conversation.modelChanges;
  }
  const change: ChatModelChange = {
    id: createId(),
    afterTurnCount,
    modelName: next.name,
    createdAt: timestamp,
  };
  const newest = conversation.modelChanges.at(-1);
  return newest?.afterTurnCount === change.afterTurnCount
    ? [...conversation.modelChanges.slice(0, -1), change]
    : [...conversation.modelChanges, change];
}

/**
 * Returns the configuration options carried by a session-scoped update.
 *
 * `null` means the update belongs to a turn. Keeping the test in one place means
 * the load replay and the live prompt stream cannot disagree about which updates
 * describe the conversation rather than its transcript.
 */
function sessionScopedConfigOptions(
  update: acp.SessionUpdate,
): acp.SessionConfigOption[] | null {
  return update.sessionUpdate === "config_option_update"
    ? update.configOptions
    : null;
}

/**
 * Streams one prompt, rebuilding a route that died under a still-open session.
 *
 * `sessionStopped` before the first event means the connection generation this
 * session was routed on is gone — an agent provider that restarted, not a
 * conversation the user ended. Ora's load path already knows how to re-attach
 * and restore the agent's context, so the send borrows it once instead of
 * surfacing a failure that the very next message would have repaired anyway.
 *
 * The reload is streamed and discarded: the caller's transcript is already the
 * session's history, so applying the replay would only repaint what is on
 * screen. Nothing is retried once an event has been delivered — a stream that
 * broke midway has already shown the user part of a turn, and sending the same
 * prompt again would duplicate it.
 */
async function* promptWithReattach(
  client: ChatSessionClient,
  sessionId: string,
  prompt: acp.ContentBlock[],
  recordPrompt: acp.ContentBlock[] | undefined,
  signal: AbortSignal,
): AsyncGenerator<PromptSessionEvent> {
  let delivered = false;
  try {
    for await (const event of client.prompt(
      {
        sessionId,
        prompt,
        ...(recordPrompt === undefined ? {} : { recordPrompt }),
      },
      { signal },
    )) {
      delivered = true;
      yield event;
    }
    return;
  } catch (error) {
    if (delivered || !isSessionStoppedError(error)) throw error;
  }
  await loadSessionConversation(client, sessionId, signal);
  yield* client.prompt(
    {
      sessionId,
      prompt,
      ...(recordPrompt === undefined ? {} : { recordPrompt }),
    },
    { signal },
  );
}
/** Reports whether a failure is the backend refusing a session that holds no live route. */
function isSessionStoppedError(error: unknown): boolean {
  return (
    error instanceof RemoteContractError && error.code === "session_stopped"
  );
}

/**
 * Loads one session's full history into a `SessionConversation` without a store.
 *
 * Read-only surfaces (workflow node sessions) reuse the same transcript
 * normalization as the chat pane without attaching a live store, so the loaded
 * conversation renders identically to the session page.
 */
export async function loadSessionConversation(
  client: ChatSessionClient,
  sessionId: string,
  signal?: AbortSignal,
): Promise<SessionConversation> {
  const staged = new HistoryBuilder(() => crypto.randomUUID(), Date.now);
  let completed = false;
  for await (const event of client.load({ sessionId }, { signal })) {
    if (event.type === "session_update") {
      const configOptions = sessionScopedConfigOptions(event.update);
      if (configOptions) {
        staged.configOptions = configOptions;
      } else {
        staged.applyUpdate(event.update);
      }
    } else if (event.type === "permission_request") {
      staged.addPermission(event);
    } else if (event.type === "turn_ended") {
      staged.endTurn(event.stopReason);
    } else {
      completed = true;
    }
  }
  if (!completed) {
    throw new Error("agent session load ended before completion");
  }
  const conversation = staged.finish();
  return {
    ...conversation,
    // Replay rebuilds the transcript from the provider, which knows nothing about
    // Ora's markers, so model-change markers are dropped rather than misplaced.
    configOptions: staged.configOptions ?? conversation.configOptions,
    modelChanges: [],
  };
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
    turns: conversation.turns.map((turn) =>
      turn.id === turnId ? update(turn) : turn,
    ),
  }));
}

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

function isAbortError(error: unknown): boolean {
  // A user-facing stop surfaces through the Desktop transport as a `cancelled`
  // LocalTransportError rather than a DOMException AbortError; both mean the
  // caller asked to stop, so they must share the graceful-cancel path and never
  // surface the transport's technical English message into the conversation.
  return (
    (error instanceof Error && error.name === "AbortError") ||
    (error instanceof LocalTransportError && error.kind === "cancelled")
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Agent request failed";
}
