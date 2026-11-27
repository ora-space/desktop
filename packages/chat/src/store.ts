import type * as acp from "@agentclientprotocol/sdk";
import {
  type ContractsClient,
  type LoadSessionEvent,
  type PromptSessionEvent,
  LocalTransportError,
  RemoteContractError,
  type SessionHistoryNotice,
  type SessionPermissionRequest,
} from "@ora/contracts";
import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  ChatContent,
  ChatMessage,
  ChatThought,
  ChatModelChange,
  ChatPlan,
  ChatToolCall,
  ChatToolCallStatus,
  ChatTurn,
  SessionConversation,
} from "./types.ts";
import { currentModel } from "./model-option.ts";
import { observeDiagnostics, type RunDiagnostics } from "./diagnostics.ts";
import {
  childSessionSource,
  extractToolMetadata,
  mergeToolMetadata,
  metadataKey,
  withItemMetadata,
  withToolMetadata,
} from "./item-metadata.ts";
import {
  applyContextUsageUpdate,
  beginUsageAfterAgentSwitch,
  beginUsageTurn,
  completeUsageTurn,
  createHiddenSessionUsage,
  createReloadedSessionUsage,
} from "./usage.ts";

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
  ChatTurnRetry,
  ChatTurnStatus,
  ContextUsageSnapshot,
  ContextUsageState,
  LastTurnTokenState,
  SessionConversation,
  SessionUsage,
} from "./types.ts";

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
   * The session to stream into. Always known before sending: a surface with no
   * session yet creates one first and streams into the id that returns.
   */
  oraSessionId: string;
  /**
   * A model chosen while this session held no provider, for the attach this send performs.
   *
   * Reading a conversation does not reach its agent, so a session can be open with nothing to
   * configure; its picker offers the agent's own catalog instead and the pick waits here. A
   * session that already reports options is configured directly and sends none.
   */
  model?: string;
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

/** What creating or rebinding the session revealed about the agent behind it. */
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
   * Used to seed a session's options before any turn exists, from a start or
   * switch response the store did not issue itself.
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
   * gets recorded. A model chosen before any session exists is client-side
   * intent instead, applied by the call that creates the session.
   */
  setSessionConfig(
    oraSessionId: string,
    configId: string,
    value: string,
  ): Promise<void>;
  loadSession(oraSessionId: string): Promise<void>;
  /**
   * Hears that a session's options may have changed, or that it has no provider now.
   *
   * Without a stream the options are read from the session; with one the stream's are taken, and
   * the session is read once the stream ends. Sessions not on screen are left alone.
   */
  optionsChanged(oraSessionId: string): void;
  /** Reads the options of every conversation on screen again, after announcements may have been lost. */
  resyncOptions(): void;
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
  /**
   * Turns a failed request into the sentence shown in the conversation.
   *
   * The store is locale-agnostic, so the host supplies the translation of contract errors;
   * without one the store falls back to the raw `Error.message`.
   */
  describeError?: (error: unknown) => string;
}

export type ChatStore = StoreApi<ChatState>;
export type ChatSessionClient = Pick<
  ContractsClient["session"],
  "getOptions" | "load" | "prompt" | "respondToPermission" | "setConfig"
>;

const EMPTY_CONVERSATION: SessionConversation = {
  configOptions: [],
  providerLost: false,
  modelChanges: [],
  historyNotices: [],
  turns: [],
  availableCommands: null,
  sessionTitle: null,
  sessionUpdatedAt: null,
  isLoaded: false,
  isLoading: false,
  isResponding: false,
  pendingPermissions: [],
  usage: createHiddenSessionUsage(),
  error: null,
};

/** Caps one live text batch so streamed output stays responsive under very large responses. */
const STREAMING_TEXT_BATCH_LIMIT = 4 * 1024;

interface BufferedTextChunk {
  metadata: unknown;
  contentContext: { metadata?: unknown; annotations?: unknown };
  itemKind: "message" | "thought";
  messageId: string | undefined;
  text: string;
  timestamp: number;
}

/** How one session's options reach the store: its streams, and the read that asks for them. */
interface OptionsSync {
  /** Streams of this session open now. */
  streams: number;
  /** Whether the streams open now brought any options. */
  streamOptions: boolean;
  /** Whether the options were announced while a stream was open. */
  announced: boolean;
  reading: boolean;
  /** Whether an announcement came while reading, so the session is read again. */
  readAgain: boolean;
  /** Advances whenever a stream starts, so a read from before it is dropped. */
  generation: number;
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
  const describeError = options.describeError ?? errorMessage;
  const operations = new Map<string, AbortController>();
  const optionsSync = new Map<string, OptionsSync>();

  /** What is known about how one session's options reach this store. */
  const syncOf = (oraSessionId: string) => {
    let sync = optionsSync.get(oraSessionId);
    if (!sync) {
      sync = {
        streams: 0,
        streamOptions: false,
        announced: false,
        reading: false,
        readAgain: false,
        generation: 0,
      };
      optionsSync.set(oraSessionId, sync);
    }
    return sync;
  };

  /**
   * Reads the options the session is on, or that it has no provider.
   *
   * One read at a time: an announcement during it reads again after. A result is dropped when a
   * stream started meanwhile, because the stream starts from the session's options and is newer.
   */
  const readOptions = (oraSessionId: string) => {
    const sync = syncOf(oraSessionId);
    if (sync.reading) {
      sync.readAgain = true;
      return;
    }
    sync.reading = true;
    sync.readAgain = false;
    const generation = sync.generation;
    void client
      .getOptions({ sessionId: oraSessionId })
      .then(
        ({ configOptions }) => {
          if (sync.generation !== generation) return;
          updateConversation(store.setState, oraSessionId, (conversation) =>
            configOptions === null
              ? { ...conversation, providerLost: true }
              : { ...conversation, configOptions, providerLost: false },
          );
        },
        // A lost read leaves the view where it was, like a lost announcement.
        () => {},
      )
      .finally(() => {
        sync.reading = false;
        if (!sync.readAgain) return;
        sync.readAgain = false;
        if (sync.streams > 0) sync.announced = true;
        else readOptions(oraSessionId);
      });
  };

  /** Notes a stream of this session starting: anything read before it is older than it. */
  const openOptionsStream = (oraSessionId: string) => {
    const sync = syncOf(oraSessionId);
    if (sync.streams === 0) {
      sync.streamOptions = false;
      sync.announced = false;
    }
    sync.streams += 1;
    sync.generation += 1;
  };

  /**
   * Takes the options a stream brought.
   *
   * No read is in flight to drop: one from before the stream was dropped when it started, and none
   * starts while a stream is open.
   */
  const takeStreamOptions = (
    oraSessionId: string,
    configOptions: acp.SessionConfigOption[],
  ) => {
    const sync = syncOf(oraSessionId);
    sync.streamOptions = true;
    updateConversation(store.setState, oraSessionId, (conversation) => ({
      ...conversation,
      configOptions,
      providerLost: false,
    }));
  };

  /**
   * Notes a stream of this session ending, and reads the options when it may have missed some.
   *
   * Announcements made during the stream were only noted, and one that brought no options at all
   * comes from a session that may have no provider: either way the session is asked now.
   */
  const closeOptionsStream = (oraSessionId: string) => {
    const sync = syncOf(oraSessionId);
    sync.streams -= 1;
    if (sync.streams > 0) return;
    if (sync.announced || !sync.streamOptions) readOptions(oraSessionId);
  };

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
      updateConversation(set, oraSessionId, (conversation) => ({
        ...withConfigOptions(
          conversation,
          configOptions,
          createId,
          now(),
          // The turn that carries the move is already on screen and is the first
          // one the incoming agent answers, so the mark belongs before it rather
          // than after the exchange it introduced.
          Math.max(conversation.turns.length - 1, 0),
        ),
        usage: beginUsageAfterAgentSwitch(),
        ...(conversation.diagnostics ? { diagnostics: undefined } : {}),
      }));
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
          error: describeError(error),
        }));
        throw error;
      }
    },

    loadSession: async (oraSessionId) => {
      if (operations.has(oraSessionId)) return;
      const previous = get().conversations[oraSessionId] ?? EMPTY_CONVERSATION;
      const reloadedUsage = createReloadedSessionUsage(
        previous.turns.length > 0,
      );
      const controller = new AbortController();
      const staged = new HistoryBuilder(createId, now);
      let completed = false;
      let pendingPreviewTimer: ReturnType<typeof setTimeout> | null = null;

      /**
       * Reads the options the conversation is on now rather than staging them with the transcript.
       *
       * A load can last as long as a running turn, and a model the user picks meanwhile reaches the
       * conversation without passing through this stream.
       */
      const liveConfigOptions = () =>
        get().conversations[oraSessionId]?.configOptions ??
        previous.configOptions;

      /** Publishes the active replay turn without repainting once per provider text token. */
      const flushPendingPreview = () => {
        if (pendingPreviewTimer !== null) {
          clearTimeout(pendingPreviewTimer);
          pendingPreviewTimer = null;
        }
        const preview = staged.preview(liveConfigOptions());
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
      openOptionsStream(oraSessionId);
      updateConversation(set, oraSessionId, () => ({
        ...previous,
        turns: [],
        usage: reloadedUsage,
        ...(previous.diagnostics ? { diagnostics: undefined } : {}),
        isLoading: true,
        error: null,
      }));
      try {
        for await (const event of staged.read(
          client,
          oraSessionId,
          controller.signal,
          describeError,
        )) {
          let batchPreview = false;
          if (event.type === "session_update") {
            // Session-scoped updates are split out before the turn accumulator
            // sees them; they describe the conversation, not any one turn.
            const configOptions = sessionScopedConfigOptions(event.update);
            if (configOptions) {
              takeStreamOptions(oraSessionId, configOptions);
            } else {
              staged.applyUpdate(
                event.update,
                recordedAtMillis(event.recordedAt),
                event.toolTiming,
              );
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
            staged.endTurn(
              event.stopReason,
              recordedAtMillis(event.recordedAt),
            );
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
          // An agent that reports nothing on load leaves whatever session
          // creation already established, rather than blanking the picker.
          configOptions: liveConfigOptions(),
          // Replay rebuilds the transcript from the provider, which knows
          // nothing about Ora's markers, so any earlier ones cannot be placed
          // against the new turns and are dropped rather than misplaced.
          modelChanges: [],
        }));
      } catch (error) {
        updateConversation(set, oraSessionId, () => ({
          ...previous,
          configOptions: liveConfigOptions(),
          usage: reloadedUsage,
          ...(previous.diagnostics ? { diagnostics: undefined } : {}),
          error: isAbortError(error) ? previous.error : describeError(error),
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
        closeOptionsStream(oraSessionId);
      }
    },

    optionsChanged: (oraSessionId) => {
      if (!get().conversations[oraSessionId]) return;
      const sync = syncOf(oraSessionId);
      if (sync.streams > 0) sync.announced = true;
      else readOptions(oraSessionId);
    },

    resyncOptions: () => {
      for (const oraSessionId of Object.keys(get().conversations)) {
        get().optionsChanged(oraSessionId);
      }
    },

    sendMessage: async ({
      oraSessionId,
      text,
      images = [],
      agentText,
      model,
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
      let usageCompleted = false;

      let pendingDiagnostics: RunDiagnostics | undefined;
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
            batch.metadata,
            batch.contentContext,
          ),
        );
        if (pendingDiagnostics) {
          const diagnostics = pendingDiagnostics;
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            diagnostics,
          }));
          pendingDiagnostics = undefined;
        }
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
            pendingTextChunk.messageId !== (chunk.messageId ?? undefined) ||
            metadataKey(
              pendingTextChunk.metadata,
              pendingTextChunk.contentContext,
            ) !==
              metadataKey(chunk._meta, {
                metadata: chunk.content._meta,
                annotations: chunk.content.annotations,
              }))
        ) {
          flushPendingTextChunk();
        }
        pendingDiagnostics = observeDiagnostics(
          pendingDiagnostics ?? get().conversations[key]?.diagnostics,
          {
            ...chunk,
            sessionUpdate:
              itemKind === "message"
                ? "agent_message_chunk"
                : "agent_thought_chunk",
          },
          now(),
        );
        if (pendingTextChunk === null) {
          pendingTextChunk = {
            metadata: chunk._meta,
            contentContext: {
              metadata: chunk.content._meta,
              annotations: chunk.content.annotations,
            },
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
        // A prompt Ora is about to stream is a live attempt: an empty snapshot
        // lets the panel say "nothing observed yet" instead of borrowing the
        // unknown wording that belongs to replayed history.
        diagnostics: {},
        isResponding: true,
        error: null,
      }));
      if (prepare) {
        let prepared: PreparedSession;
        try {
          prepared = await prepare();
        } catch (error) {
          // Preparation failed before the prompt phase began, so there is no response duration.
          const message = describeError(error);
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
          updateConversation(set, key, (conversation) =>
            conversation.diagnostics?.provider
              ? {
                  ...conversation,
                  diagnostics: {
                    ...conversation.diagnostics,
                    provider: undefined,
                  },
                }
              : conversation,
          );
          throw error;
        }
        // Written before a stop is honoured: the session is already on this agent, so whatever an
        // earlier one offered is gone even if no prompt follows. Agents usually report after
        // answering setup, so a setup that collected none has not said there are none.
        updateConversation(set, key, (conversation) => ({
          ...conversation,
          availableCommands:
            prepared.availableCommands.length > 0
              ? prepared.availableCommands
              : null,
        }));
        if (controller.signal.aborted) {
          // Stopped mid-startup: the session exists but we never open its stream.
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming"
              ? {
                  ...current,
                  status: "cancelled",
                }
              : current,
          );
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            isResponding: false,
          }));
          operations.delete(key);
          updateConversation(set, key, (conversation) =>
            conversation.diagnostics?.provider
              ? {
                  ...conversation,
                  diagnostics: {
                    ...conversation.diagnostics,
                    provider: undefined,
                  },
                }
              : conversation,
          );
          return;
        }
        updateTurn(set, key, turnId, (current) => ({
          ...current,
          responseStartedAt: now(),
        }));
        // This turn was streamed live, so the local conversation already is the
        // session's history. Marking it loaded stops the workspace's "load if not
        // loaded" effect from firing once the session becomes selectable — that
        // reload clears turns to empty first, which would bounce the composer back
        // to the landing layout and replay the slide-down animation.
        updateConversation(set, key, (conversation) => ({
          ...conversation,
          isLoaded: true,
        }));
      }

      updateConversation(set, key, (conversation) => ({
        ...conversation,
        usage: beginUsageTurn(conversation.usage),
      }));

      openOptionsStream(key);
      try {
        for await (const event of promptWithReattach(
          client,
          key,
          prompt,
          recordPrompt,
          model,
          controller.signal,
        )) {
          if (event.type === "session_update") {
            // The user turn is already materialized, so the echoed prompt chunk
            // would only duplicate it; every other update belongs to this turn.
            const update = event.update;
            if (update.sessionUpdate === "user_message_chunk") continue;
            if (!(
              (update.sessionUpdate === "agent_message_chunk" ||
                update.sessionUpdate === "agent_thought_chunk") &&
              update.content.type === "text"
            )) {
              flushPendingTextChunk();
              updateConversation(set, key, (conversation) => {
                const diagnostics = observeDiagnostics(
                  conversation.diagnostics,
                  update,
                  now(),
                );
                return diagnostics === conversation.diagnostics
                  ? conversation
                  : { ...conversation, diagnostics };
              });
            }
            if (update.sessionUpdate === "usage_update") {
              updateConversation(set, key, (conversation) => ({
                ...conversation,
                usage: applyContextUsageUpdate(
                  conversation.usage,
                  update,
                  now(),
                ),
              }));
              continue;
            }
            // An agent may change its own configuration mid-turn; that describes
            // the session, so it never reaches the turn accumulator.
            const configOptions = sessionScopedConfigOptions(update);
            if (configOptions) {
              takeStreamOptions(key, configOptions);
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
              applyAgentUpdate(
                current,
                update,
                createId,
                now(),
                event.toolTiming,
              ),
            );
          } else if (event.type === "permission_request") {
            flushPendingTextChunk();
            appendPermission(set, key, event);
          } else if (event.type === "retrying") {
            flushPendingTextChunk();
            // The stalled attempt was cancelled before the re-send, so tools it
            // left open are interrupted now rather than ticking until the turn
            // ends; its pending permissions were answered by that cancel too.
            const retriedAt = now();
            updateConversation(set, key, (conversation) => ({
              ...conversation,
              // The re-sent attempt is still observed live; only the cancelled
              // attempt's activity stops counting as evidence for this window.
              diagnostics: {},
            }));
            updateTurn(set, key, turnId, (current) => ({
              ...settleActiveToolCalls(current, "cancelled", retriedAt),
              retry: { retry: event.retry, maxRetries: event.maxRetries },
            }));
            clearPendingPermissions(set, key);
          } else {
            flushPendingTextChunk();
            usageCompleted = true;
            const completedAt = now();
            updateTurn(set, key, turnId, (current) =>
              settleActiveToolCalls(
                {
                  ...current,
                  status:
                    event.stopReason === "cancelled"
                      ? ("cancelled" as const)
                      : ("completed" as const),
                  stopReason: event.stopReason,
                  durationMs: elapsedDuration(
                    current.responseStartedAt ?? current.createdAt,
                    now(),
                  ),
                },
                impliedToolStatus(event.stopReason),
                completedAt,
              ),
            );
            updateConversation(set, key, (conversation) => ({
              ...conversation,
              // A turn the Agent finished is its chance to report; a cancelled one may never
              // have reached it.
              availableCommands:
                conversation.availableCommands ??
                (event.stopReason === "cancelled" ? null : []),
              usage: completeUsageTurn(
                conversation.usage,
                event.tokenUsage,
                completedAt,
              ),
            }));
          }
        }
      } catch (error) {
        flushPendingTextChunk();
        if (isAbortError(error)) {
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming"
              ? settleActiveToolCalls(
                  {
                    ...current,
                    status: "cancelled",
                    durationMs: elapsedDuration(
                      current.responseStartedAt ?? current.createdAt,
                      now(),
                    ),
                  },
                  "cancelled",
                  now(),
                )
              : current,
          );
          clearPendingPermissions(set, key);
        } else {
          const message = describeError(error);
          // A timeout after the backend already re-sent the prompt means every
          // retry stalled too; the turn keeps that so the UI can say the agent
          // never came back rather than showing a generic failure.
          const retriesExhausted = isAgentTimedOutError(error);
          // The failure ended the turn, so tools the agent never settled were
          // interrupted by it. They are not marked failed: the stream broke, and
          // whether the tool itself succeeded is exactly what was never reported.
          updateTurn(set, key, turnId, (current) =>
            current.status === "streaming"
              ? settleActiveToolCalls(
                  {
                    ...current,
                    status: "failed",
                    error: message,
                    ...(retriesExhausted && current.retry !== undefined
                      ? { retry: { ...current.retry, exhausted: true } }
                      : {}),
                    durationMs: elapsedDuration(
                      current.responseStartedAt ?? current.createdAt,
                      now(),
                    ),
                  },
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
        closeOptionsStream(key);
        updateConversation(set, key, (conversation) =>
          conversation.diagnostics?.provider
            ? {
                ...conversation,
                diagnostics: {
                  ...conversation.diagnostics,
                  provider: undefined,
                },
              }
            : conversation,
        );
        if (!usageCompleted) {
          updateConversation(set, key, (conversation) => ({
            ...conversation,
            usage: completeUsageTurn(conversation.usage, undefined, now()),
          }));
        }
        // A stream that ended without a boundary event still closes the turn, so
        // its tools settle with it rather than outliving the turn that owns them.
        // Nothing reported them finishing, so they close as interrupted.
        updateTurn(set, key, turnId, (current) =>
          current.status === "streaming"
            ? settleActiveToolCalls(
                {
                  ...current,
                  status: "completed",
                  durationMs: elapsedDuration(
                    current.responseStartedAt ?? current.createdAt,
                    now(),
                  ),
                },
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
          error: describeError(error),
        }));
        throw error;
      }
    },

    clearAll: () => {
      optionsSync.clear();
      set({ conversations: {} });
    },
    dispose: () => {
      operations.forEach((controller) => controller.abort());
      operations.clear();
      optionsSync.clear();
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
  private availableCommands: acp.AvailableCommand[] | null = null;
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
  private readonly recordedTurnStarts = new Map<string, number>();

  constructor(
    private readonly createId: () => string,
    private readonly now: () => number,
  ) {}

  /** A follower timeout immediately after its live turn boundary belongs to that turn.
   * All other errors still abort the load, including errors during historical replay.
   */
  async *read(
    client: ChatSessionClient,
    sessionId: string,
    signal?: AbortSignal,
    describeError: (error: unknown) => string = errorMessage,
  ): AsyncGenerator<LoadSessionEvent> {
    let endedLiveTurn = false;
    try {
      for await (const event of client.load({ sessionId }, { signal })) {
        endedLiveTurn =
          event.type === "turn_ended" &&
          event.stopReason === "cancelled" &&
          event.recordedAt == null;
        yield event;
      }
    } catch (error) {
      const last = this.turns.at(-1);
      if (!endedLiveTurn || !last || !isAgentTimedOutError(error)) throw error;
      this.replaceLast({
        ...last,
        status: "failed",
        error: describeError(error),
      });
      this.permissions.length = 0;
      yield { type: "completed" };
    }
  }

  applyUpdate(
    update: acp.SessionUpdate,
    recordedAt?: number,
    toolTiming?: import("@ora/contracts").ToolCallTiming,
  ): void {
    if (update.sessionUpdate === "user_message_chunk") {
      this.appendUserChunk(update, recordedAt);
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
    const turn = this.currentTurn(recordedAt);
    this.replaceLast(
      applyAgentUpdate(
        turn,
        update,
        this.createId,
        this.timestamp(recordedAt),
        toolTiming,
        true,
      ),
    );
  }

  addPermission(request: SessionPermissionRequest): void {
    this.permissions.push(request);
  }

  /** Retains one backend-authored integrity notice without guessing a turn position. */
  addHistoryNotice(notice: SessionHistoryNotice): void {
    this.historyNotices.push(notice);
  }

  /** Settles the open turn with the outcome the record captured for it. */
  endTurn(stopReason: acp.StopReason, recordedAt?: number): void {
    const last = this.turns.at(-1);
    this.hasOpenTurn = false;
    if (last === undefined) return;
    const settled: ChatTurn = {
      ...last,
      status: stopReason === "cancelled" ? "cancelled" : "completed",
      stopReason,
      ...(recordedAt === undefined || !this.recordedTurnStarts.has(last.id)
        ? {}
        : {
            durationMs: elapsedDuration(
              this.recordedTurnStarts.get(last.id)!,
              recordedAt,
            ),
          }),
    };
    this.replaceLast(
      settleActiveToolCalls(
        settled,
        impliedToolStatus(stopReason),
        this.timestamp(recordedAt),
      ),
    );
  }

  /** Produces a complete loaded conversation after the finite replay stream ends. */
  finish(): SessionConversation {
    const turns = this.turns.map((turn) =>
      turn.status === "streaming"
        ? settleActiveToolCalls(
            { ...turn, status: "completed" as const },
            "cancelled",
            this.now(),
          )
        : turn,
    );
    return {
      ...this.snapshot(),
      // A turn still streaming here never reached its boundary in the record,
      // which is what an interrupted process leaves behind. Its tools close with
      // it as interrupted too, so replay neither restores work that appears to
      // still be running nor credits it with an outcome the record never held.
      turns,
      usage: createReloadedSessionUsage(turns.length > 0),
      pendingPermissions: this.permissions,
      isLoaded: true,
    };
  }

  /** Materializes safe partial replay state while a load stream is still producing events. */
  preview(configOptions: acp.SessionConfigOption[]): SessionConversation {
    return {
      ...this.snapshot(),
      configOptions,
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
      usage: createReloadedSessionUsage(this.turns.length > 0),
    };
  }

  private appendUserChunk(chunk: acp.ContentChunk, recordedAt?: number): void {
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
    const createdAt = this.timestamp(recordedAt);
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
    if (recordedAt !== undefined) {
      this.recordedTurnStarts.set(this.turns.at(-1)!.id, recordedAt);
    }
    this.hasOpenTurn = true;
  }

  /** Ensures an agent update always has a turn, even before any user message replays. */
  private currentTurn(recordedAt?: number): ChatTurn {
    const last = this.turns.at(-1);
    if (this.hasOpenTurn && last !== undefined) return last;
    const createdAt = this.timestamp(recordedAt);
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

  /** Prefers the history line's time when replay supplied one. */
  private timestamp(recordedAt?: number): number {
    return recordedAt ?? this.now();
  }
}

/** Normalizes one agent-produced ACP update into a response turn's ordered items. */
function applyAgentUpdate(
  turn: ChatTurn,
  update: acp.SessionUpdate,
  createId: () => string,
  timestamp: number,
  toolTiming?: import("@ora/contracts").ToolCallTiming,
  trustTurnEndSettled = false,
): ChatTurn {
  switch (update.sessionUpdate) {
    case "agent_message_chunk":
      return appendContentChunk(turn, "message", update, createId, timestamp);
    case "agent_thought_chunk":
      return appendContentChunk(turn, "thought", update, createId, timestamp);
    case "plan":
      return replacePlan(turn, update.entries, timestamp, update._meta);
    case "tool_call":
      return upsertToolCall(
        turn,
        update,
        timestamp,
        toolTiming,
        trustTurnEndSettled,
      );
    case "tool_call_update":
      return updateToolCall(
        turn,
        update,
        timestamp,
        toolTiming,
        trustTurnEndSettled,
      );
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
    const item: ChatContent = {
      kind: "content",
      id: createId(),
      source: itemKind,
      content,
      createdAt: timestamp,
    };
    return {
      ...turn,
      items: [...turn.items, withItemMetadata(item, chunk._meta)],
    };
  }

  return appendTextContentChunk(
    turn,
    itemKind,
    chunk.messageId ?? undefined,
    content.text,
    timestamp,
    chunk._meta,
    { metadata: content._meta, annotations: content.annotations },
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
  parent: string | undefined,
): number {
  if (messageId !== undefined) {
    return turn.items.findIndex(
      (item) =>
        item.kind === itemKind &&
        item.protocolMessageId === messageId &&
        metadataKey(item.metadata, item.contentContext) === parent,
    );
  }
  for (let index = turn.items.length - 1; index >= 0; index -= 1) {
    const item = turn.items[index]!;
    if (item.kind !== "message" && item.kind !== "thought") return -1;
    if (
      item.kind === itemKind &&
      metadataKey(item.metadata, item.contentContext) !== parent
    )
      return -1;
    if (
      item.kind === itemKind &&
      item.protocolMessageId === undefined &&
      metadataKey(item.metadata, item.contentContext) === parent
    )
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
  metadata?: unknown,
  contentContext?: { metadata?: unknown; annotations?: unknown },
): ChatTurn {
  const parent = metadataKey(metadata, contentContext);
  const itemIndex = textRunIndex(turn, itemKind, messageId, parent);
  if (itemIndex === -1) {
    // An agent switching to a tool call commonly emits empty or newline-only
    // deltas first. Opening a run on those would place a blank block ahead of
    // the work that follows it, so an ordinary run starts where its content does. Attributed
    // chunks retain even leading whitespace so a source switch cannot silently discard content.
    if (text.trim() === "" && parent === undefined) return turn;
    // Implicit runs are numbered by where they begin, which keeps them distinct
    // now that one turn can hold several.
    const sourceSuffix =
      parent === undefined ? "" : `:source:${turn.items.length}`;
    const itemId =
      messageId === undefined
        ? `${itemKind}-implicit-${turn.id}-${turn.items.length}`
        : `${itemKind}-${messageId}${sourceSuffix}`;
    const item: ChatMessage | ChatThought =
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
    return {
      ...turn,
      items: [...turn.items, withItemMetadata(item, metadata, contentContext)],
    };
  }

  const items = [...turn.items];
  const item = items[itemIndex]!;
  if (item.kind === "message" || item.kind === "thought") {
    items[itemIndex] = withItemMetadata(
      { ...item, content: item.content + text },
      metadata,
      contentContext,
    );
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

/** Replaces the current turn's complete plan snapshot per source without changing its timeline position. */
function replacePlan(
  turn: ChatTurn,
  entries: acp.PlanEntry[],
  timestamp: number,
  metadata?: unknown,
): ChatTurn {
  const sourceId = childSessionSource(metadata)?.id;
  const planIndex = turn.items.findIndex(
    (item) =>
      item.kind === "plan" &&
      childSessionSource(item.metadata)?.id === sourceId,
  );
  if (planIndex === -1) {
    const plan: ChatPlan = {
      kind: "plan",
      id: `plan-${turn.id}${sourceId === undefined ? "" : `:source:${turn.items.length}`}`,
      entries,
      createdAt: timestamp,
      updatedAt: timestamp,
    };
    return {
      ...turn,
      items: [...turn.items, withItemMetadata(plan, metadata)],
    };
  }

  const items = [...turn.items];
  const plan = items[planIndex] as ChatPlan;
  items[planIndex] = withItemMetadata(
    { ...plan, entries, updatedAt: timestamp },
    metadata,
  );
  return { ...turn, items };
}

/** Inserts a new tool call or replaces its complete initial snapshot. */
function upsertToolCall(
  turn: ChatTurn,
  toolCall: acp.ToolCall,
  timestamp: number,
  timing?: import("@ora/contracts").ToolCallTiming,
  trustTurnEndSettled = false,
): ChatTurn {
  const toolIndex = turn.items.findIndex(
    (item) => item.kind === "toolCall" && item.id === toolCall.toolCallId,
  );
  const { metadata, settledByTurnEnd } = extractToolMetadata(
    toolCall._meta,
    trustTurnEndSettled,
  );
  const next = withToolMetadata(
    {
      kind: "toolCall",
      id: toolCall.toolCallId,
      title: toolCall.title,
      ...(toolCall.kind === undefined ? {} : { toolKind: toolCall.kind }),
      ...(toolCall.status === undefined ? {} : { status: toolCall.status }),
      ...(settledByTurnEnd ? { settledByTurnEnd: true } : {}),
      content: toolCall.content ?? [],
      locations: toolCall.locations ?? [],
      ...(toolCall.rawInput === undefined
        ? {}
        : { rawInput: toolCall.rawInput }),
      ...(toolCall.rawOutput === undefined
        ? {}
        : { rawOutput: toolCall.rawOutput }),
      createdAt:
        toolIndex === -1
          ? timestamp
          : (turn.items[toolIndex] as ChatToolCall).createdAt,
      updatedAt: timestamp,
      ...toolTimingFields(timing),
    },
    metadata,
  );
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
  timing?: import("@ora/contracts").ToolCallTiming,
  trustTurnEndSettled = false,
): ChatTurn {
  const toolIndex = turn.items.findIndex(
    (item) => item.kind === "toolCall" && item.id === update.toolCallId,
  );
  const { metadata: incomingMeta, settledByTurnEnd } = extractToolMetadata(
    update._meta,
    trustTurnEndSettled,
  );
  if (toolIndex === -1) {
    const tool = withToolMetadata(
      {
        kind: "toolCall",
        id: update.toolCallId,
        title: update.title ?? "Tool call",
        ...(update.kind === undefined || update.kind === null
          ? {}
          : { toolKind: update.kind }),
        ...(update.status === undefined || update.status === null
          ? {}
          : { status: update.status }),
        ...(settledByTurnEnd ? { settledByTurnEnd: true } : {}),
        content: update.content ?? [],
        locations: update.locations ?? [],
        ...(update.rawInput === undefined ? {} : { rawInput: update.rawInput }),
        ...(update.rawOutput === undefined
          ? {}
          : { rawOutput: update.rawOutput }),
        createdAt: timestamp,
        updatedAt: timestamp,
        ...toolTimingFields(timing),
      },
      mergeToolMetadata(undefined, incomingMeta),
    );
    return { ...turn, items: [...turn.items, tool] };
  }

  const items = [...turn.items];
  const { settledByTurnEnd: previousSettledByTurnEnd, ...current } = items[
    toolIndex
  ] as ChatToolCall;
  const keepSettledByTurnEnd =
    settledByTurnEnd ||
    (previousSettledByTurnEnd === true &&
      (update.status === undefined || update.status === null));
  items[toolIndex] = withToolMetadata(
    {
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
      ...(keepSettledByTurnEnd ? { settledByTurnEnd: true } : {}),
      ...(update.content === undefined
        ? {}
        : { content: update.content ?? [] }),
      ...(update.locations === undefined
        ? {}
        : { locations: update.locations ?? [] }),
      ...(update.rawInput === undefined ? {} : { rawInput: update.rawInput }),
      ...(update.rawOutput === undefined
        ? {}
        : { rawOutput: update.rawOutput }),
      updatedAt: timestamp,
      ...toolTimingFields(timing),
    },
    mergeToolMetadata(current.metadata, incomingMeta),
  );
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
    items: turn.items.map((item) => {
      if (
        item.kind !== "toolCall" ||
        (item.status !== "pending" && item.status !== "in_progress")
      ) {
        return item;
      }
      const nextStatus =
        childSessionSource(item.metadata) !== undefined ? "unknown" : status;
      return {
        ...item,
        status: nextStatus,
        ...(nextStatus === "completed" ? { settledByTurnEnd: true } : {}),
        updatedAt: timestamp,
        ...(item.startedAt === undefined
          ? {}
          : { durationMs: elapsedDuration(item.startedAt, timestamp) }),
      };
    }),
  };
}

/** Converts optional transport timing without falling back to UI receipt timestamps. */
function toolTimingFields(
  timing: import("@ora/contracts").ToolCallTiming | undefined,
): Pick<ChatToolCall, "startedAt" | "durationMs"> {
  if (timing === undefined) return {};
  const startedAt = Date.parse(timing.startedAt);
  if (!Number.isFinite(startedAt)) return {};
  const durationMs =
    timing.durationMs === undefined ? undefined : Number(timing.durationMs);
  return {
    startedAt,
    ...(durationMs === undefined ||
    !Number.isFinite(durationMs) ||
    durationMs < 0
      ? {}
      : { durationMs }),
  };
}

/** Returns a validated non-negative duration between two epoch-millisecond boundaries. */
function elapsedDuration(startedAt: number, finishedAt: number): number {
  return Math.max(0, finishedAt - startedAt);
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
  model: string | undefined,
  signal: AbortSignal,
): AsyncGenerator<PromptSessionEvent> {
  let delivered = false;
  try {
    for await (const event of client.prompt(
      {
        sessionId,
        prompt,
        ...(recordPrompt === undefined ? {} : { recordPrompt }),
        ...(model === undefined ? {} : { model }),
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
      ...(model === undefined ? {} : { model }),
    },
    { signal },
  );
}
/** Reports whether a failure is the backend giving up on a prompt that made no progress. */
function isAgentTimedOutError(error: unknown): boolean {
  return (
    error instanceof RemoteContractError && error.code === "agent_timed_out"
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
  for await (const event of staged.read(client, sessionId, signal)) {
    if (event.type === "session_update") {
      const configOptions = sessionScopedConfigOptions(event.update);
      if (configOptions) {
        staged.configOptions = configOptions;
      } else {
        staged.applyUpdate(
          event.update,
          recordedAtMillis(event.recordedAt),
          event.toolTiming,
        );
      }
    } else if (event.type === "permission_request") {
      staged.addPermission(event);
    } else if (event.type === "turn_ended") {
      staged.endTurn(event.stopReason, recordedAtMillis(event.recordedAt));
    } else if (event.type === "history_notice") {
      staged.addHistoryNotice(event.notice);
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

/** Parses a history-line RFC 3339 stamp. Invalid or missing values fall through to `now()`. */
function recordedAtMillis(recordedAt: string | undefined): number | undefined {
  if (recordedAt === undefined || recordedAt.length === 0) {
    return undefined;
  }
  const parsed = Date.parse(recordedAt);
  return Number.isNaN(parsed) ? undefined : parsed;
}
