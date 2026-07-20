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

/** Configures deterministic mock ACP timing and identity generation. */
export interface MockAcpClientOptions {
  scheduler?: MockAcpScheduler;
  chunkSize?: number;
  chunkDelayMs?: number;
  createId?: () => string;
  initialSessionIds?: Iterable<string>;
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

      const promptText = request.prompt
        .filter(isTextContent)
        .map((block) => block.text)
        .join("\n");
      const response = `Mock response: ${promptText}`;
      const messageId = `agent-message-${createId()}`;
      activePrompts.add(request.sessionId);

      try {
        for (const text of splitText(response, chunkSize)) {
          await scheduler.wait(chunkDelayMs);
          emit(listeners, {
            sessionId: request.sessionId,
            update: {
              sessionUpdate: "agent_message_chunk",
              messageId,
              content: { type: "text", text },
            },
          });
        }
        return { stopReason: "end_turn" };
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
