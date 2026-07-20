import type { acp } from "@ora/contracts";

/** Receives one protocol update emitted by an ACP agent. */
export type AcpSessionNotificationListener = (
  notification: acp.SessionNotification,
) => void;

/**
 * Defines the transport-independent ACP operations required by the chat domain.
 * Implementations own wire details and deliver agent updates through subscriptions.
 */
export interface AcpClient {
  newSession(request: acp.NewSessionRequest): Promise<acp.NewSessionResponse>;
  prompt(request: acp.PromptRequest): Promise<acp.PromptResponse>;
  subscribe(listener: AcpSessionNotificationListener): () => void;
}

/** Creates an ACP client that fails explicitly until a real transport is configured. */
export function createUnavailableAcpClient(): AcpClient {
  const unavailable = (): never => {
    throw new Error("ACP transport is not configured");
  };

  return {
    newSession: async () => unavailable(),
    prompt: async () => unavailable(),
    subscribe: () => () => undefined,
  };
}
