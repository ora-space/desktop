import type { acp } from "@ora/contracts";
import type { AcpClient } from "./client.js";

/** Configures one reusable baseline ACP client behavior check. */
export interface AcpClientConformanceOptions {
  newSessionRequest: acp.NewSessionRequest;
  prompt: acp.ContentBlock[];
  cancelAfterFirstUpdate?: boolean;
}

/** Captures the observable result of one conformance turn. */
export interface AcpClientConformanceResult {
  session: acp.NewSessionResponse;
  response: acp.PromptResponse;
  notifications: acp.SessionNotification[];
}

/**
 * Exercises only behavior shared by mocks and real transports: session identity,
 * update delivery, unsubscription, and optional cancellation after the first update.
 */
export async function exerciseAcpClientConformance(
  client: AcpClient,
  options: AcpClientConformanceOptions,
): Promise<AcpClientConformanceResult> {
  const session = await client.newSession(options.newSessionRequest);
  if (session.sessionId.trim() === "") throw new Error("ACP client returned an empty session id");

  const notifications: acp.SessionNotification[] = [];
  let cancellation: Promise<void> | null = null;
  const unsubscribe = client.subscribe((notification) => {
    if (notification.sessionId !== session.sessionId) {
      throw new Error("ACP client delivered an update for the wrong session");
    }
    notifications.push(notification);
    if (options.cancelAfterFirstUpdate && cancellation === null) {
      cancellation = client.cancel({ sessionId: session.sessionId });
    }
  });

  try {
    const response = await client.prompt({ sessionId: session.sessionId, prompt: options.prompt });
    await cancellation;
    if (notifications.length === 0) throw new Error("ACP client completed without delivering an update");
    if (options.cancelAfterFirstUpdate && response.stopReason !== "cancelled") {
      throw new Error("ACP client did not report cancellation after a cancel notification");
    }
    return { session, response, notifications };
  } finally {
    unsubscribe();
  }
}
