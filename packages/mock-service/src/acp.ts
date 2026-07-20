import type {
  AcpClient,
  AcpSessionNotificationListener,
} from "@ora/chat";
import type { acp } from "@ora/contracts";

const DEFAULT_CHUNK_SIZE = 8;
const DEFAULT_CHUNK_DELAY_MS = 80;
const SEEDED_AGENT_SESSION_ID = "agent-session-runtime";

/** Waits between mock chunks so production-like streaming remains testable. */
export interface MockAcpScheduler {
  wait(delayMs: number): Promise<void>;
}

/**
 * Selects a failure the mock agent injects into `prompt`.
 *
 * A real ACP agent fails in ways a purely happy-path mock cannot express, and
 * callers that only ever see a clean `end_turn` grow logic that breaks on first
 * contact with a real transport. Each variant models one such class of failure.
 */
export type MockAcpFault =
  /** Rejects before any chunk is emitted, as an unreachable agent would. */
  | { kind: "failBeforeStream"; message: string }
  /** Emits `afterChunks` chunks and then rejects, as a dropped stream would. */
  | { kind: "failMidStream"; afterChunks: number; message: string }
  /**
   * Streams the full reply but never delivers the turn-end response, modelling a
   * transport that silently stalls. The returned promise never settles.
   */
  | { kind: "hang" };

/** Configures deterministic mock ACP timing, identity generation, and failures. */
export interface MockAcpClientOptions {
  scheduler?: MockAcpScheduler;
  chunkSize?: number;
  chunkDelayMs?: number;
  createId?: () => string;
  initialSessionIds?: Iterable<string>;
  /** Turn-end reason for successful prompts; real agents use all five values. */
  stopReason?: acp.StopReason;
  fault?: MockAcpFault;
}

const timeoutScheduler: MockAcpScheduler = {
  wait: (delayMs) =>
    new Promise((resolve) => {
      setTimeout(resolve, delayMs);
    }),
};

/** Creates an in-memory ACP agent that streams deterministic text replies. */
export function createMockAcpClient(
  options: MockAcpClientOptions = {},
): AcpClient {
  const scheduler = options.scheduler ?? timeoutScheduler;
  const chunkSize = options.chunkSize ?? DEFAULT_CHUNK_SIZE;
  const chunkDelayMs = options.chunkDelayMs ?? DEFAULT_CHUNK_DELAY_MS;
  const createId = options.createId ?? (() => crypto.randomUUID());
  const stopReason = options.stopReason ?? "end_turn";
  const fault = options.fault;
  const sessionIds = new Set(
    options.initialSessionIds ?? [SEEDED_AGENT_SESSION_ID],
  );
  const activePrompts = new Set<string>();
  const listeners = new Set<AcpSessionNotificationListener>();

  if (!Number.isInteger(chunkSize) || chunkSize <= 0) {
    throw new Error("mock ACP chunkSize must be a positive integer");
  }

  return {
    async newSession(_request) {
      const sessionId = `agent-session-${createId()}`;
      sessionIds.add(sessionId);
      return { sessionId };
    },

    async prompt(request) {
      if (!sessionIds.has(request.sessionId)) {
        throw new Error(`ACP session not found: ${request.sessionId}`);
      }
      if (activePrompts.has(request.sessionId)) {
        throw new Error(`ACP session is already processing a prompt: ${request.sessionId}`);
      }

      if (fault?.kind === "failBeforeStream") {
        throw new Error(fault.message);
      }

      const promptText = request.prompt
        .filter(isTextContent)
        .map((block) => block.text)
        .join("\n");
      const response = `Mock response: ${promptText}`;
      const messageId = `agent-message-${createId()}`;
      activePrompts.add(request.sessionId);

      try {
        const chunks = splitText(response, chunkSize);
        for (const [index, text] of chunks.entries()) {
          await scheduler.wait(chunkDelayMs);
          emit(listeners, {
            sessionId: request.sessionId,
            update: {
              sessionUpdate: "agent_message_chunk",
              messageId,
              content: { type: "text", text },
            },
          });
          if (fault?.kind === "failMidStream" && index + 1 >= fault.afterChunks) {
            throw new Error(fault.message);
          }
        }

        if (fault?.kind === "hang") {
          // Deliberately never settles, so the session stays active and the
          // `finally` below never runs. That is the point: it reproduces a
          // stalled turn that no timeout on our side would otherwise reveal.
          await new Promise<never>(() => undefined);
        }

        return { stopReason };
      } finally {
        activePrompts.delete(request.sessionId);
      }
    },

    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

/** Narrows prompt blocks to the baseline text content supported by this mock. */
function isTextContent(
  block: acp.ContentBlock,
): block is Extract<acp.ContentBlock, { type: "text" }> {
  return block.type === "text";
}

/** Splits text into stable chunks without losing whitespace or punctuation. */
function splitText(text: string, chunkSize: number): string[] {
  const chunks: string[] = [];
  for (let offset = 0; offset < text.length; offset += chunkSize) {
    chunks.push(text.slice(offset, offset + chunkSize));
  }
  return chunks;
}

/** Delivers one session update to a snapshot so listeners may unsubscribe safely. */
function emit(
  listeners: Set<AcpSessionNotificationListener>,
  notification: acp.SessionNotification,
): void {
  [...listeners].forEach((listener) => listener(notification));
}
